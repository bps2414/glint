//! Publicar pela tela do clipe e gerenciar o link publicado.
//!
//! O rodapé do painel mostra o andamento (login → preparar → enviar), com
//! cancelar; no fim o link vai para a área de transferência. Com um jogo aberto
//! o envio pausa sozinho e retoma quando ele fecha ("Enviar mesmo assim" ignora o
//! jogo). Publicar com outro envio em andamento põe o clipe na fila ([`Queued`]),
//! inclusive vários de uma vez pela seleção da biblioteca.
//!
//! "Exportar MP4" salva o mesmo vídeo do publicar num arquivo ([`crate::exporter`]), sem
//! servidor nem conta. A marca d'água vem das Configurações e pode mudar só para o clipe.

use std::path::{Path, PathBuf};
use std::time::Duration;

use app_cloud::{clip_id_from_url, login, Client, CloudError, Visibility};
use app_core::{brand, library::Clip, Config};
use app_media::ExportOptions;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Shell;
use crate::assets::icon;
use crate::components::{field_label, glass, input_box, Btn, BtnKind, Segmented, Toggle};
use crate::exporter::{Export, State as ExportState};
use crate::format;
use crate::publisher::{Failure, Job, Phase, Request};
use crate::recorder::Shown;
use crate::secret;
use crate::theme::size::Size;
use crate::theme::{tok, FONT_NUM, R_PANEL, R_SM};
use crate::{tr, tr_args};

/// Altura máxima do vídeo publicado.
const PUBLISH_MAX_HEIGHT: u32 = 1080;

/// Visibilidade do clipe no servidor, como a tela do clipe conhece.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Remote {
    /// Sem conta ou ainda não perguntado.
    #[default]
    Unknown,
    Loading,
    Ready(Visibility),
    /// Mudando para esta.
    Saving(Visibility),
    /// O servidor não tem (apagado pelo site ou é de outra conta).
    Missing,
}

/// Clipe esperando a vez na fila de publicação.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Queued {
    pub clip_id: i64,
    pub vis: Visibility,
}

impl Shell {
    /// Jogo sendo gravado agora (gravação de tela manual não conta).
    fn game_open(&self) -> Option<String> {
        match Shown::of(self.status.as_ref()) {
            Shown::Recording(Some(g)) => Some(g),
            _ => None,
        }
    }

    /// Tem servidor configurado (sem ele nada de publicar, conta ou fila).
    pub fn has_server(&self) -> bool {
        !self.config.server_url.trim().is_empty()
    }

    /// Sem servidor: abre "Entrar num servidor" (o link do convite traz o endereço).
    pub fn ask_server(&mut self, cx: &mut Context<Self>) {
        self.open_join(None, cx);
    }

    fn publishing(&self) -> bool {
        self.job.as_ref().is_some_and(|j| j.phase.running())
    }

    /// Algo da publicação deste clipe na tela (andamento ou erro).
    pub fn publish_busy(&self, id: i64) -> bool {
        self.job.as_ref().is_some_and(|j| j.clip_id == id)
    }

    /// Publicação deste clipe em andamento (não conta a que falhou).
    fn publish_running(&self, id: i64) -> bool {
        self.job
            .as_ref()
            .is_some_and(|j| j.clip_id == id && j.phase.running())
    }

    /// "Exportar MP4" deste clipe em andamento.
    pub fn exporting(&self, id: i64) -> bool {
        self.export
            .as_ref()
            .is_some_and(|e| e.clip_id == id && e.running())
    }

    /// Marca d'água deste clipe: a das Configurações, salvo se o painel trocou.
    pub fn mark_on(&self, id: i64) -> bool {
        self.watermark_on
            .get(&id)
            .copied()
            .unwrap_or(self.config.watermark)
    }

    fn toggle_mark(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.clip.as_ref().map(|v| v.id) else {
            return;
        };
        // O vídeo em preparo já saiu com a escolha anterior.
        if self.publish_running(id) || self.exporting(id) {
            return;
        }
        let on = !self.mark_on(id);
        if on == self.config.watermark {
            self.watermark_on.remove(&id);
        } else {
            self.watermark_on.insert(id, on);
        }
        cx.notify();
    }

    /// O vídeo que sai do clipe (publicar e exportar): trecho, trilhas ligadas e marca.
    /// Mudar qualquer coisa aqui faz o publicar exportar de novo.
    fn export_opts(&self, c: &Clip) -> ExportOptions {
        ExportOptions {
            start_ms: c.trim_start_ms.unwrap_or(0),
            end_ms: c.trim_end_ms.unwrap_or(0),
            tracks: self.track_on(c.id).to_vec(),
            max_height: PUBLISH_MAX_HEIGHT,
            watermark: if self.mark_on(c.id) {
                crate::watermark::watermark(self.config.watermark_corner)
            } else {
                None
            },
        }
    }

    /// Posição na fila (1 = o próximo).
    pub fn queue_pos(&self, id: i64) -> Option<usize> {
        self.queue
            .iter()
            .position(|q| q.clip_id == id)
            .map(|i| i + 1)
    }

    /// Botão "Publicar" (e "Tentar de novo"). Com outro clipe enviando, entra na fila.
    pub fn publish_clicked(&mut self, cx: &mut Context<Self>) {
        if !self.has_server() {
            self.ask_server(cx);
            return;
        }
        let Some((id, vis)) = self.clip.as_ref().map(|v| (v.id, v.vis)) else {
            return;
        };
        if self.publishing() && self.publish_busy(id) {
            return;
        }
        self.save_title(cx);
        self.save_trim();
        if self.publishing() {
            self.enqueue(id, vis);
            cx.notify();
            return;
        }
        self.start_publish(id, vis, cx);
    }

    fn enqueue(&mut self, id: i64, vis: Visibility) {
        if self.queue_pos(id).is_none() && !self.publish_busy(id) {
            self.queue.push(Queued { clip_id: id, vis });
        }
    }

    pub fn unqueue(&mut self, id: i64, cx: &mut Context<Self>) {
        self.queue.retain(|q| q.clip_id != id);
        cx.notify();
    }

    /// Publica os clipes selecionados na biblioteca (os já publicados ficam de fora).
    pub fn publish_selected(&mut self, cx: &mut Context<Self>) {
        if !self.has_server() {
            self.ask_server(cx);
            return;
        }
        let ids: Vec<i64> = self
            .store
            .clips
            .iter()
            .filter(|c| self.selected.contains(&c.id) && c.published_url.is_none())
            .map(|c| c.id)
            .collect();
        self.selected.clear();
        for id in ids {
            self.enqueue(id, Visibility::Unlisted);
        }
        if !self.publishing() {
            self.next_in_queue(cx);
        }
        cx.notify();
    }

    /// Começa o próximo da fila (pula os que sumiram ou já foram publicados).
    fn next_in_queue(&mut self, cx: &mut Context<Self>) {
        while !self.queue.is_empty() {
            let q = self.queue.remove(0);
            let ok = self
                .store
                .index_of(q.clip_id)
                .is_some_and(|i| self.store.clips[i].published_url.is_none());
            if ok {
                self.start_publish(q.clip_id, q.vis, cx);
                return;
            }
        }
    }

    /// A cada status do gravador: pausa o envio com jogo aberto, solta quando fecha.
    pub fn sync_pause(&mut self) {
        let game = self.game_open().is_some();
        if let Some(j) = self.job.as_ref().filter(|j| j.phase.running()) {
            j.set_paused(game && !j.forced);
        }
    }

    /// "Enviar mesmo assim": não pausa mais este envio por causa do jogo.
    fn send_anyway(&mut self, cx: &mut Context<Self>) {
        if let Some(j) = self.job.as_mut() {
            j.forced = true;
            j.set_paused(false);
        }
        cx.notify();
    }

    fn start_publish(&mut self, id: i64, vis: Visibility, cx: &mut Context<Self>) {
        if !self.has_server() {
            return;
        }
        let Some(c) = self.store.index_of(id).map(|i| self.store.clips[i].clone()) else {
            return;
        };
        let base = self.config.server_url.clone();
        let login = if self.token.is_none() {
            match login::start(&base) {
                Ok(p) => {
                    cx.open_url(&p.url);
                    Some(p)
                }
                Err(e) => {
                    self.notify_user(
                        tr_args("publish.login_error", &[("error", &e.to_string())]),
                        cx,
                    );
                    return;
                }
            }
        } else {
            None
        };
        let req = Request {
            clip_id: id,
            src: c.path.clone(),
            opts: self.export_opts(&c),
            title: publish_title(&c),
            game: c.game.clone(),
            visibility: vis,
            base,
            token: self.token.clone(),
            login,
            login_page: login_page(),
        };
        let previous = self.job.take().filter(|j| !j.phase.running());
        let dir = Config::data_dir().join("publicar");
        let paused = self.game_open().is_some();
        self.job = Some(Job::start(req, &dir, previous, paused));
        self.watch_publish(cx);
        cx.notify();
    }

    fn watch_publish(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            match this.update(cx, |t, cx| t.on_publish_tick(cx)) {
                Ok(true) => {}
                _ => break,
            }
        })
        .detach();
    }

    /// `true` = continuar acompanhando.
    fn on_publish_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((changed, token)) = self.job.as_mut().map(|j| j.poll()) else {
            return false;
        };
        if let Some(t) = token {
            secret::save(&t);
            self.token = Some(t);
            self.fetch_account(cx);
        }
        let Some(job) = self.job.as_mut() else {
            return false;
        };
        if !changed {
            return true;
        }
        let id = job.clip_id;
        match job.phase.clone() {
            Phase::Done(url) => {
                self.job = None;
                self.store.set_published(id, Some(&url));
                if let Some(v) = self.clip.as_mut().filter(|v| v.id == id) {
                    v.remote = Remote::Ready(v.vis);
                }
                self.copy_link(id, cx);
                self.notify_user(tr("publish.done"), cx);
                self.next_in_queue(cx);
                false
            }
            Phase::Failed(Failure::Cancelled) => {
                if let Some(j) = self.job.take() {
                    j.discard(self.token.clone());
                }
                self.next_in_queue(cx);
                cx.notify();
                false
            }
            // Sem conta a fila para: o próximo abriria o navegador sem ninguém pedir.
            Phase::Failed(Failure::Session) => {
                self.session_expired();
                cx.notify();
                false
            }
            // Servidor cheio: os da fila também falhariam, então ela é desfeita.
            Phase::Failed(Failure::StorageFull {
                used_bytes,
                limit_bytes,
            }) => {
                self.queue.clear();
                self.notify_user(storage_full_text(used_bytes, limit_bytes), cx);
                cx.notify();
                false
            }
            Phase::Failed(Failure::Error(e)) => {
                if !self.queue.is_empty() {
                    // O erro some quando o próximo começa: fica no aviso do rodapé.
                    self.notify_user(tr_args("publish.failed", &[("error", &e)]), cx);
                    self.next_in_queue(cx);
                }
                cx.notify();
                false
            }
            _ => {
                cx.notify();
                true
            }
        }
    }

    fn session_expired(&mut self) {
        secret::delete();
        self.token = None;
    }

    fn cancel_publish(&mut self, cx: &mut Context<Self>) {
        if let Some(j) = &self.job {
            j.cancel();
        }
        cx.notify();
    }

    fn dismiss_publish(&mut self, cx: &mut Context<Self>) {
        if let Some(j) = self.job.take() {
            j.discard(self.token.clone());
        }
        cx.notify();
    }

    fn reopen_login(&self, cx: &mut Context<Self>) {
        if let Some(url) = self.job.as_ref().and_then(|j| j.login_url.clone()) {
            cx.open_url(&url);
        }
    }

    /// Id no servidor do clipe publicado `id`.
    fn server_id(&self, id: i64) -> Option<String> {
        let i = self.store.index_of(id)?;
        let url = self.store.clips[i].published_url.as_deref()?;
        clip_id_from_url(url).map(str::to_string)
    }

    pub(crate) fn client(&self) -> Option<Client> {
        if !self.has_server() {
            return None;
        }
        let token = self.token.clone()?;
        Some(Client::new(&self.config.server_url, Some(token)))
    }

    /// Pergunta ao servidor a visibilidade do clipe aberto (se publicado).
    pub fn fetch_remote(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.clip.as_ref().map(|v| v.id) else {
            return;
        };
        let (Some(sid), Some(client)) = (self.server_id(id), self.client()) else {
            return;
        };
        if let Some(v) = self.clip.as_mut() {
            v.remote = Remote::Loading;
        }
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { client.get_clip(&sid) })
                .await;
            let _ = this.update(cx, |t, cx| {
                let remote = match r {
                    Ok((c, _)) => Remote::Ready(c.visibility),
                    Err(CloudError::Api { status: 404, .. }) => Remote::Missing,
                    Err(CloudError::Unauthorized) => {
                        t.session_expired();
                        Remote::Unknown
                    }
                    Err(e) => {
                        tracing::warn!("visibilidade do clipe: {e}");
                        Remote::Unknown
                    }
                };
                if let Some(v) = t.clip.as_mut().filter(|v| v.id == id) {
                    v.remote = remote;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn set_visibility(&mut self, vis: Visibility, cx: &mut Context<Self>) {
        let Some(v) = self.clip.as_ref() else { return };
        let (id, before) = (v.id, v.remote);
        let Remote::Ready(old) = before else { return };
        if old == vis {
            return;
        }
        let (Some(sid), Some(client)) = (self.server_id(id), self.client()) else {
            return;
        };
        if let Some(v) = self.clip.as_mut() {
            v.remote = Remote::Saving(vis);
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { client.update_clip(&sid, None, Some(vis)) })
                .await;
            let _ = this.update(cx, |t, cx| {
                let remote = match r {
                    Ok(c) => Remote::Ready(c.visibility),
                    Err(e) => {
                        if matches!(e, CloudError::Unauthorized) {
                            t.session_expired();
                        }
                        t.notify_user(
                            tr_args("publish.vis_error", &[("error", &e.to_string())]),
                            cx,
                        );
                        before
                    }
                };
                if let Some(v) = t.clip.as_mut().filter(|v| v.id == id) {
                    v.remote = remote;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn ask_unpublish(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(v) = self.clip.as_mut() {
            v.confirm_unpublish = on;
            cx.notify();
        }
    }

    /// Apaga do servidor; o clipe continua no PC.
    pub fn unpublish(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.clip.as_ref().map(|v| v.id) else {
            return;
        };
        if let Some(v) = self.clip.as_mut() {
            v.confirm_unpublish = false;
        }
        let (Some(sid), Some(client)) = (self.server_id(id), self.client()) else {
            return;
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { client.delete_clip(&sid) })
                .await;
            let _ = this.update(cx, |t, cx| match r {
                Ok(()) | Err(CloudError::Api { status: 404, .. }) => t.forget_link(id, cx),
                Err(e) => {
                    if matches!(e, CloudError::Unauthorized) {
                        t.session_expired();
                    }
                    t.notify_user(
                        tr_args("publish.unpublish_error", &[("error", &e.to_string())]),
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// "Exportar MP4": pergunta onde salvar (diálogo do sistema) e exporta em segundo plano.
    fn export_clicked(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.clip.as_ref().map(|v| v.id) else {
            return;
        };
        if self.export.as_ref().is_some_and(|e| e.running()) {
            return;
        }
        self.save_title(cx);
        self.save_trim();
        let Some(src) = self
            .store
            .index_of(id)
            .map(|i| self.store.clips[i].path.clone())
        else {
            return;
        };
        let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
        let stem = export_stem(&src);
        let picked = cx.prompt_for_new_path(&dir, Some(&format!("{stem}.mp4")));
        cx.spawn(async move |this, cx| {
            let picked = picked.await;
            let _ = this.update(cx, |t, cx| match picked {
                Ok(Ok(Some(path))) => t.start_export(id, path, cx),
                Ok(Ok(None)) => {}
                // Sem diálogo de salvar: vai ao lado do clipe (o aviso do fim mostra o caminho).
                other => {
                    if let Ok(Err(e)) = other {
                        tracing::warn!("diálogo de salvar: {e:#}");
                    }
                    let path = app_core::naming::unique_path(&dir, &stem, "mp4");
                    t.start_export(id, path, cx);
                }
            });
        })
        .detach();
    }

    fn start_export(&mut self, id: i64, mut dst: PathBuf, cx: &mut Context<Self>) {
        if self.export.as_ref().is_some_and(|e| e.running()) {
            return;
        }
        let Some(c) = self.store.index_of(id).map(|i| self.store.clips[i].clone()) else {
            return;
        };
        if dst.extension().is_none() {
            dst.set_extension("mp4");
        }
        if same_file(&dst, &c.path) {
            self.notify_user(tr("export.same_file"), cx);
            return;
        }
        let opts = self.export_opts(&c);
        self.export = Some(Export::start(id, c.path.clone(), dst, opts));
        self.watch_export(cx);
        cx.notify();
    }

    fn watch_export(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            match this.update(cx, |t, cx| t.on_export_tick(cx)) {
                Ok(true) => {}
                _ => break,
            }
        })
        .detach();
    }

    /// `true` = continuar acompanhando.
    fn on_export_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(e) = self.export.as_mut() else {
            return false;
        };
        if !e.poll() {
            return true;
        }
        let (state, dst) = (e.state.clone(), e.dst.display().to_string());
        match state {
            ExportState::Running(_) => {
                cx.notify();
                return true;
            }
            ExportState::Done => {
                self.notify_user(tr_args("export.done", &[("path", &dst)]), cx);
            }
            ExportState::Failed(err) => {
                self.notify_user(tr_args("export.failed", &[("error", &err)]), cx);
            }
            ExportState::Cancelled => {}
        }
        self.export = None;
        cx.notify();
        false
    }

    fn cancel_export(&mut self, cx: &mut Context<Self>) {
        if let Some(e) = &self.export {
            e.cancel();
        }
        cx.notify();
    }

    /// Tira o link do clipe local (já não existe no servidor).
    fn forget_link(&mut self, id: i64, cx: &mut Context<Self>) {
        self.store.set_published(id, None);
        if let Some(v) = self.clip.as_mut().filter(|v| v.id == id) {
            v.remote = Remote::Unknown;
        }
        self.notify_user(tr("publish.unpublished"), cx);
    }

    // ---------- desenho ----------

    /// Painel de vidro à direita da tela do clipe: título, quem pode ver, o que vai ser
    /// enviado e, embaixo, o botão principal (ou o andamento) e a conta.
    pub fn publish_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let v = self.clip.as_ref().expect("clipe aberto");
        let c = self
            .store
            .index_of(v.id)
            .map(|i| self.store.clips[i].clone());
        let published = c.as_ref().is_some_and(|c| c.published_url.is_some());
        let count = v.title.read(cx).value().chars().count();

        let title = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .child(field_label(tr("clip.title")))
                    .child(
                        div()
                            .font_family(FONT_NUM)
                            .text_size(px(11.))
                            .text_color(tok::muted())
                            .child(format!("{count}/{}", crate::views::clip::TITLE_MAX)),
                    ),
            )
            .child(input_box(&v.title, window, cx));

        glass::glass_panel(R_PANEL)
            .flex()
            .flex_col()
            .flex_none()
            .w(px(300.))
            .h_full()
            .child(
                div()
                    .id("pub-body")
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(18.))
                    .pt(px(20.))
                    .pb(px(8.))
                    .child(
                        div()
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr(if published {
                                "publish.heading_done"
                            } else {
                                "publish.heading"
                            })),
                    )
                    .child(title)
                    .children(self.link_section(c.as_ref(), cx))
                    .when(!published && self.has_server(), |d| {
                        d.child(self.choose_visibility(cx))
                    })
                    .when(!published, |d| {
                        d.children(c.as_ref().map(|c| self.export_info(c)))
                    })
                    .children(c.as_ref().map(|c| self.mark_row(c.id, cx))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .flex_none()
                    .px(px(18.))
                    .pt(px(8.))
                    .pb(px(20.))
                    .child(self.publish_footer(c.as_ref(), cx))
                    .children(c.as_ref().map(|c| self.export_row(c.id, cx)))
                    .when(self.has_server(), |d| d.child(self.account_row(cx))),
            )
    }

    /// "Quem pode ver" antes de publicar (vai junto no envio).
    fn choose_visibility(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let vis = self.clip.as_ref().map_or(Visibility::Unlisted, |v| v.vis);
        let busy = self.clip.as_ref().is_some_and(|v| self.publish_busy(v.id));
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(field_label(tr("publish.visibility")))
            .child(
                Segmented::new("pub-vis-new", vis_labels(), vis_index(vis))
                    .disabled(busy)
                    .on_select(cx.listener(|this, i: &usize, _, cx| {
                        if let Some(v) = this.clip.as_mut() {
                            v.vis = VISIBILITIES[*i];
                            cx.notify();
                        }
                    })),
            )
            .child(hint(tr(vis_hint_key(vis))))
    }

    /// O que vai ser enviado: formato, som e se é o trecho ou o clipe inteiro.
    fn export_info(&self, c: &Clip) -> impl IntoElement {
        let v = self.clip.as_ref().expect("clipe aberto");
        let on = self.track_on(c.id);
        let n = v.tracks.unwrap_or(c.audio_tracks as usize);
        let (game, mic) = (n >= 1 && on[0], n >= 2 && on[1]);
        let audio = tr(match (game, mic) {
            (true, true) => "publish.audio_both",
            (true, false) => "publish.audio_game",
            (false, true) => "publish.audio_mic",
            (false, false) => "publish.audio_none",
        });
        let height = c.height.min(PUBLISH_MAX_HEIGHT);
        let trimmed = v.trim_ms() != (None, None);
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(tok::fg_2())
            .child(tr_args(
                "publish.info",
                &[
                    ("format", &format!("{height}p{}", c.fps)),
                    ("audio", &audio),
                ],
            ))
            .child(tr(if trimmed {
                "publish.trimmed"
            } else {
                "publish.whole"
            }))
    }

    /// "Marca d'água" só para este clipe (começa no valor das Configurações).
    fn mark_row(&self, id: i64, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.publish_running(id) || self.exporting(id);
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(field_label(tr("export.watermark")))
                    .child(
                        div().when(busy, |d| d.opacity(0.6)).child(
                            Toggle::new("pub-mark", self.mark_on(id))
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_mark(cx))),
                        ),
                    ),
            )
            .child(hint(tr("export.watermark_hint")))
    }

    /// "Exportar MP4" (ou o andamento, com cancelar). Um por vez; não precisa de conta.
    fn export_row(&self, id: i64, cx: &mut Context<Self>) -> AnyElement {
        if self.exporting(id) {
            let f = match self.export.as_ref().map(|e| &e.state) {
                Some(ExportState::Running(f)) => *f,
                _ => 1.0,
            };
            return div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(progress_row(
                    tr("export.exporting"),
                    format!("{:.0}%", f.clamp(0.0, 1.0) * 100.0),
                    f,
                ))
                .child(
                    Btn::new("exp-cancel", BtnKind::Secondary, tr("clip.cancel"))
                        .full_width()
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_export(cx))),
                )
                .into_any_element();
        }
        let other = self.export.as_ref().is_some_and(|e| e.running());
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .when(other, |d| d.child(hint(tr("export.other_running"))))
            .child(
                Btn::new("clip-export", BtnKind::Secondary, tr("export.button"))
                    .full_width()
                    .disabled(other)
                    .on_click(cx.listener(|this, _, _, cx| this.export_clicked(cx))),
            )
            .into_any_element()
    }

    /// Conta embaixo do painel: "Conectado como X · Sair", ou o aviso do login.
    fn account_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .min_h(Size::Sm.h())
            .text_size(px(12.))
            .text_color(tok::fg_2());
        if self.token.is_none() {
            return row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .line_height(px(17.))
                    .child(tr("publish.signed_out")),
            );
        }
        let name = self.account.clone();
        let initial: String = name
            .as_deref()
            .and_then(|n| n.chars().find(|c| c.is_alphanumeric()))
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default();
        row.child(
            div()
                .flex_none()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(tok::field())
                .border_1()
                .border_color(tok::edge())
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tok::fg())
                .child(initial),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .gap(px(4.))
                .child(tr("publish.signed_in_as"))
                .children(name.map(|n| {
                    div()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(tok::fg())
                        .child(n)
                })),
        )
        .child(
            Btn::new("pub-sign-out", BtnKind::Ghost, tr("settings.sign_out"))
                .size(Size::Sm)
                .disabled(self.publishing())
                .on_click(cx.listener(|this, _, _, cx| this.sign_out(cx))),
        )
    }

    /// Botão principal (Publicar / Copiar link) ou o andamento do envio.
    fn publish_footer(&self, c: Option<&Clip>, cx: &mut Context<Self>) -> AnyElement {
        let Some(c) = c else {
            return Empty.into_any_element();
        };
        let id = c.id;
        let col = || div().flex().flex_col().gap(px(12.));

        if let Some(j) = self.job.as_ref().filter(|j| j.clip_id == id) {
            return self.job_status(j, cx);
        }
        if let Some(pos) = self.queue_pos(id) {
            return col()
                .child(status_text(
                    tr("publish.in_queue"),
                    Some(tr_args(
                        "publish.in_queue_hint",
                        &[("ahead", &pos.to_string())],
                    )),
                ))
                .child(
                    Btn::new("pub-unqueue", BtnKind::Secondary, tr("publish.unqueue"))
                        .full_width()
                        .on_click(cx.listener(move |this, _, _, cx| this.unqueue(id, cx))),
                )
                .into_any_element();
        }

        if c.published_url.is_none() && !self.has_server() {
            return col()
                .child(status_text(
                    tr("publish.no_server"),
                    Some(tr("publish.no_server_hint")),
                ))
                .child(
                    Btn::new(
                        "pub-connect",
                        BtnKind::Primary,
                        tr("publish.connect_server"),
                    )
                    .size(Size::Lg)
                    .full_width()
                    .on_click(cx.listener(|this, _, _, cx| this.ask_server(cx))),
                )
                .into_any_element();
        }

        let other = self.publishing();
        let copied = self.copied == Some(id);
        let primary = if c.published_url.is_some() {
            Btn::new(
                "clip-copy",
                BtnKind::Primary,
                tr(if copied {
                    "card.copied"
                } else {
                    "card.copy_link"
                }),
            )
            .icon(if copied { icon::CHECK } else { icon::LINK })
            .on_click(cx.listener(move |this, _, _, cx| this.copy_link(id, cx)))
        } else if other {
            Btn::new("clip-publish", BtnKind::Primary, tr("publish.add_queue"))
                .icon(icon::UPLOAD)
                .on_click(cx.listener(|this, _, _, cx| this.publish_clicked(cx)))
        } else {
            Btn::new("clip-publish", BtnKind::Primary, tr("publish.publish_copy"))
                .icon(icon::LINK)
                .on_click(cx.listener(|this, _, _, cx| this.publish_clicked(cx)))
        };
        col()
            .when(other && c.published_url.is_none(), |d| {
                d.child(hint(tr("publish.other_running")))
            })
            .child(primary.size(Size::Lg).full_width())
            .into_any_element()
    }

    fn job_status(&self, j: &Job, cx: &mut Context<Self>) -> AnyElement {
        let cancel = || {
            Btn::new("pub-cancel", BtnKind::Secondary, tr("clip.cancel"))
                .full_width()
                .on_click(cx.listener(|this, _, _, cx| this.cancel_publish(cx)))
        };
        let progress = progress_row;
        let done_line = |text: SharedString| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .text_size(px(13.))
                .text_color(tok::fg_2())
                .child(
                    svg()
                        .path(icon::CHECK)
                        .size(Size::Sm.icon())
                        .text_color(tok::fg_2()),
                )
                .child(text)
        };
        let percent = |f: f32| format!("{:.0}%", f.clamp(0.0, 1.0) * 100.0);
        let col = || div().flex().flex_col().gap(px(12.));
        if j.paused && j.phase.running() {
            let game = self.game_open().unwrap_or_default();
            return col()
                .child(status_text(
                    tr_args("publish.paused", &[("game", &game)]),
                    Some(tr("publish.paused_hint")),
                ))
                .child(
                    Btn::new("pub-anyway", BtnKind::Secondary, tr("publish.send_anyway"))
                        .full_width()
                        .on_click(cx.listener(|this, _, _, cx| this.send_anyway(cx))),
                )
                .child(cancel())
                .into_any_element();
        }
        match &j.phase {
            Phase::Login => col()
                .child(status_text(
                    tr("publish.login"),
                    Some(tr("publish.login_hint")),
                ))
                .child(
                    Btn::new("pub-reopen", BtnKind::Ghost, tr("publish.reopen"))
                        .full_width()
                        .on_click(cx.listener(|this, _, _, cx| this.reopen_login(cx))),
                )
                .child(cancel())
                .into_any_element(),
            Phase::Export(f) => col()
                .child(progress(tr("publish.exporting"), percent(*f), *f))
                .child(cancel())
                .into_any_element(),
            Phase::Upload { sent, total } => col()
                .child(done_line(tr("publish.prepared")))
                .child(progress(
                    tr("publish.sending"),
                    tr_args(
                        "publish.sent_of",
                        &[
                            ("sent", &format::size(*sent)),
                            ("total", &format::size(*total)),
                        ],
                    )
                    .to_string(),
                    *sent as f32 / (*total).max(1) as f32,
                ))
                .child(cancel())
                .into_any_element(),
            Phase::Finishing | Phase::Done(_) | Phase::Failed(Failure::Cancelled) => col()
                .child(done_line(tr("publish.prepared")))
                .child(progress(tr("publish.finishing"), percent(1.0), 1.0))
                .into_any_element(),
            Phase::Failed(f) => {
                let (text, again) = match f {
                    Failure::Session => (tr("publish.session"), tr("publish.login_again")),
                    Failure::StorageFull {
                        used_bytes,
                        limit_bytes,
                    } => (
                        storage_full_text(*used_bytes, *limit_bytes),
                        tr("publish.retry"),
                    ),
                    Failure::Error(e) => (
                        tr_args("publish.failed", &[("error", e)]),
                        tr("publish.retry"),
                    ),
                    Failure::Cancelled => unreachable!(),
                };
                col()
                    .child(status_text(text, None))
                    .child(
                        Btn::new("pub-retry", BtnKind::Primary, again)
                            .icon(icon::UPLOAD)
                            .size(Size::Lg)
                            .full_width()
                            .on_click(cx.listener(|this, _, _, cx| this.publish_clicked(cx))),
                    )
                    .child(
                        Btn::new("pub-dismiss", BtnKind::Ghost, tr("publish.dismiss"))
                            .full_width()
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss_publish(cx))),
                    )
                    .into_any_element()
            }
        }
    }

    /// Bloco do link publicado: endereço (clique copia), quem pode ver (no servidor) e
    /// apagar do servidor.
    fn link_section(&self, c: Option<&Clip>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = c?;
        let url = c.published_url.clone()?;
        let id = c.id;
        let remote = self.clip.as_ref().map(|v| v.remote).unwrap_or_default();
        let shown_url = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string();
        let copied = self.copied == Some(id);

        let link_box = div()
            .id("pub-link")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .h(Size::Md.h())
            .pl(px(14.))
            .pr(px(4.))
            .bg(tok::field())
            .border_1()
            .border_color(tok::edge())
            .rounded(R_SM)
            .cursor_pointer()
            .hover(|s| s.border_color(tok::line_2()))
            .on_click(cx.listener(move |this, _, _, cx| this.copy_link(id, cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(FONT_NUM)
                    .text_size(px(12.))
                    .text_color(tok::fg_2())
                    .child(shown_url),
            )
            .child(
                div()
                    .flex_none()
                    .size(Size::Sm.h())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        svg()
                            .path(if copied { icon::CHECK } else { icon::COPY })
                            .size(Size::Sm.icon())
                            .text_color(if copied { tok::fg() } else { tok::fg_2() }),
                    ),
            );

        let section = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(field_label(tr("publish.link")))
            .child(link_box);

        if self.token.is_none() {
            return Some(
                section
                    .child(hint(tr("publish.login_to_manage")))
                    .into_any_element(),
            );
        }
        if remote == Remote::Missing {
            return Some(
                section
                    .child(hint(tr("publish.missing")))
                    .child(
                        div().child(
                            Btn::new("pub-forget", BtnKind::Ghost, tr("publish.forget"))
                                .size(Size::Sm)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.forget_link(id, cx)),
                                ),
                        ),
                    )
                    .into_any_element(),
            );
        }

        let current = match remote {
            Remote::Ready(v) | Remote::Saving(v) => Some(v),
            _ => None,
        };
        let busy = !matches!(remote, Remote::Ready(_));
        let vis = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(field_label(tr("publish.visibility")))
            .child(
                Segmented::new(
                    "pub-vis",
                    vis_labels(),
                    current.map_or(usize::MAX, vis_index),
                )
                .disabled(busy)
                .on_select(
                    cx.listener(|this, i: &usize, _, cx| this.set_visibility(VISIBILITIES[*i], cx)),
                ),
            )
            .when_some(current, |d, v| d.child(hint(tr(vis_hint_key(v)))))
            .child(
                div().child(
                    Btn::new("pub-unpublish", BtnKind::Ghost, tr("publish.unpublish"))
                        .size(Size::Sm)
                        .icon(icon::TRASH)
                        .on_click(cx.listener(|this, _, _, cx| this.ask_unpublish(true, cx))),
                ),
            );
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(18.))
                .child(section)
                .child(vis)
                .into_any_element(),
        )
    }
}

/// Ordem do "Quem pode ver".
const VISIBILITIES: [Visibility; 3] = [
    Visibility::Private,
    Visibility::Unlisted,
    Visibility::Public,
];

fn vis_index(v: Visibility) -> usize {
    VISIBILITIES.iter().position(|&x| x == v).unwrap_or(1)
}

fn vis_labels() -> Vec<SharedString> {
    VISIBILITIES.iter().map(|&v| tr(vis_key(v))).collect()
}

fn vis_key(v: Visibility) -> &'static str {
    match v {
        Visibility::Private => "publish.vis_private",
        Visibility::Unlisted => "publish.vis_unlisted",
        Visibility::Public => "publish.vis_public",
    }
}

fn vis_hint_key(v: Visibility) -> &'static str {
    match v {
        Visibility::Private => "publish.vis_private_hint",
        Visibility::Unlisted => "publish.vis_unlisted_hint",
        Visibility::Public => "publish.vis_public_hint",
    }
}

/// Título enviado: o do usuário, senão o jogo, senão a data.
fn publish_title(c: &Clip) -> String {
    let own = c.title.as_deref().map(str::trim).filter(|t| !t.is_empty());
    match (own, c.game.as_deref()) {
        (Some(t), _) => t.chars().take(crate::views::clip::TITLE_MAX).collect(),
        (None, Some(g)) => g.to_string(),
        (None, None) => tr_args(
            "publish.default_title",
            &[("date", &format::full_date(c.created_ms))],
        )
        .to_string(),
    }
}

/// Página que o navegador mostra depois do login (mesmo visual do site: fundo escuro,
/// símbolo da marca, texto neutro).
pub(crate) fn login_page() -> String {
    format!(
        r#"<!doctype html><html lang="pt-BR"><head><meta charset="utf-8"><title>{name}</title></head>
<body style="margin:0;min-height:100vh;display:grid;place-items:center;background:#0c0d10;color:#f5f5f7;font:16px/1.5 system-ui,-apple-system,'Segoe UI',sans-serif">
<main style="text-align:center;padding:24px"><div style="width:56px;height:56px;margin:0 auto 20px">{icon}</div>
<h1 style="font-size:22px;font-weight:600;letter-spacing:-0.02em;margin:0 0 6px">{title}</h1><p style="margin:0;color:rgba(235,235,245,0.68)">{body}</p></main></body></html>"#,
        name = brand::APP_NAME,
        icon = include_str!("../../../../docs/brand/icone.svg")
            .replace(r#"width="1024" height="1024""#, r#"width="56" height="56""#),
        title = tr("publish.page_title"),
        body = tr_args("publish.page_body", &[("name", brand::APP_NAME)]),
    )
}

/// Aviso de servidor cheio, com o que está usado do limite.
fn storage_full_text(used_bytes: u64, limit_bytes: u64) -> SharedString {
    tr_args(
        "publish.storage_full",
        &[
            ("used", &format::size(used_bytes)),
            ("limit", &format::size(limit_bytes)),
        ],
    )
}

/// Rótulo, valor (ex.: "42%") e a barra de progresso.
fn progress_row(label: SharedString, value: String, frac: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .flex_row()
                .items_baseline()
                .gap(px(8.))
                .text_size(px(13.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(label),
                )
                .child(
                    div()
                        .font_family(FONT_NUM)
                        .text_color(tok::fg_2())
                        .child(value),
                ),
        )
        .child(bar(frac))
}

/// Nome sugerido do MP4 exportado: o do clipe com um sufixo (sem extensão).
fn export_stem(src: &Path) -> String {
    let stem = src.file_stem().unwrap_or_default().to_string_lossy();
    format!("{stem}-{}", tr("export.file_suffix"))
}

/// O destino é o próprio clipe (exportar por cima dele estragaria o original).
fn same_file(dst: &Path, src: &Path) -> bool {
    match (std::fs::canonicalize(dst), std::fs::canonicalize(src)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn bar(frac: f32) -> Div {
    div()
        .h(px(6.))
        .w_full()
        .rounded_full()
        .bg(tok::track())
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(tok::blue())
                .w(relative(frac.clamp(0.0, 1.0))),
        )
}

fn status_text(title: SharedString, sub: Option<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(13.))
                .line_height(px(18.))
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
        .when_some(sub, |d, s| d.child(hint(s)))
}

fn hint(text: SharedString) -> Div {
    div()
        .text_size(px(12.5))
        .line_height(px(18.))
        .text_color(tok::muted())
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::publish_title;
    use app_core::library::{Clip, ClipKind};

    #[test]
    fn titulo_publicado() {
        rust_i18n::set_locale("pt-BR");
        let mut c = Clip {
            id: 1,
            path: "x.mp4".into(),
            created_ms: 0,
            duration_ms: 1000,
            size_bytes: 1,
            width: 2,
            height: 2,
            fps: 60,
            codec: "hevc".into(),
            audio_tracks: 1,
            game: Some("VALORANT".into()),
            game_exe: None,
            title: Some("  ".into()),
            thumb_path: None,
            favorite: false,
            published_url: None,
            trim_start_ms: None,
            trim_end_ms: None,
            kind: ClipKind::Replay,
            kills: None,
        };
        assert_eq!(publish_title(&c), "VALORANT");
        c.title = Some("Ace".into());
        assert_eq!(publish_title(&c), "Ace");
        c.title = None;
        c.game = None;
        assert!(publish_title(&c).starts_with("Clipe de "));
    }
}
