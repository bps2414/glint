//! View raiz: menu lateral + barra superior + biblioteca (ou configurações).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use app_core::library::ClipKind;
use app_core::{brand, Config};
use app_ipc::RecorderStatus;
use app_media::{HwPreference, Player};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::icon;
use crate::components::glass;
use crate::components::window_controls::{drag_area, WindowControls};
use crate::components::{Btn, BtnKind, GameItem, NavItem, RecStatus, SearchField, StorageMeter};
use crate::data::{self, Filter, Sort, Store};
use crate::exporter::Export;
use crate::format;
use crate::instance::Inbox;
use crate::player::audio::MAX_TRACKS;
use crate::player::{AudioOut, VideoPlayer};
use crate::publisher::Job;
use crate::recorder::{self, Shown};
use crate::secret;
use crate::theme::size::Size;
use crate::theme::size::INSET;
use crate::theme::{self, tok, FONT_UI, R_PANEL, R_SM};
use crate::thumbs::{self, Thumbs};
use crate::updater::UpdateState;
use crate::views::clip::ClipView;
use crate::views::publish::Queued;
use crate::views::server::{JoinView, ServerModal};
use crate::views::settings::SettingsView;
use crate::{tr, tr_args};

/// Símbolo da marca (asset em `assets.rs`).
const BRAND_MARK: &str = "brand/glint.svg";
/// Tempo parado em cima do card até o clipe começar a tocar nele.
const PREVIEW_DELAY: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Library,
    Settings,
}

pub struct Shell {
    pub screen: Screen,
    pub focus: FocusHandle,
    pub store: Store,
    pub thumbs: Thumbs,
    pub filter: Filter,
    pub sort: Sort,
    pub hovered: Option<i64>,
    /// Clipe tocando mudo no card sob o mouse.
    pub preview: Option<(i64, VideoPlayer)>,
    pub search: Entity<InputState>,
    pub config: Config,
    /// Último status do gravador (`None` = fechado).
    pub status: Option<RecorderStatus>,
    /// `clips_saved` do último status: quando muda, a lista recarrega.
    clips_saved: Option<u64>,
    /// Clipe cujo link acabou de ser copiado (mostra "Link copiado" no card).
    pub copied: Option<i64>,
    /// Aviso curto no rodapé (erros de ação).
    pub notice: Option<SharedString>,
    starting_recorder: bool,
    /// Clipe aberto (tela do clipe por cima da biblioteca).
    pub clip: Option<ClipView>,
    /// Saída de som, aberta no primeiro clipe.
    pub audio: Option<Rc<AudioOut>>,
    pub audio_tried: bool,
    /// Trilhas ligadas por clipe (só nesta sessão).
    pub track_on: HashMap<i64, [bool; MAX_TRACKS]>,
    /// Marca d'água por clipe, quando difere da das Configurações (só nesta sessão).
    pub watermark_on: HashMap<i64, bool>,
    pub muted: bool,
    pub fullscreen: bool,
    /// Token da conta no servidor (`None` = publicar pede login).
    pub token: Option<String>,
    /// Publicação em andamento (ou que falhou e espera "Tentar de novo").
    pub job: Option<Job>,
    /// Clipes esperando a vez de publicar.
    pub queue: Vec<Queued>,
    /// "Exportar MP4" em andamento (um por vez).
    pub export: Option<Export>,
    /// Clipes selecionados na biblioteca (Ctrl+clique).
    pub selected: HashSet<i64>,
    /// Campos da tela de Configurações (criados ao abrir).
    pub settings: Option<SettingsView>,
    /// Nome da conta no servidor (`None` = sem conta ou ainda não perguntado).
    pub account: Option<String>,
    /// Último movimento do mouse no player (a cápsula some em tela cheia).
    pub pointer_at: Instant,
    /// Versão nova do app (card na sidebar).
    pub update: UpdateState,
    /// Links de outras execuções do app (instância única).
    inbox: Inbox,
    /// Janela "Entrar num servidor".
    pub join: Option<JoinView>,
    /// Assistente "Criar meu servidor" / "Atualizar servidor".
    pub create: Option<crate::views::create_server::CreateView>,
    /// Janela "Reportar bug / Enviar feedback".
    pub feedback: Option<crate::views::feedback::FeedbackView>,
    /// Código de outro PC ou membros (abertos nas Configurações).
    pub server_modal: Option<ServerModal>,
    /// `GET /info` do servidor configurado.
    pub server_info: Option<app_cloud::ServerInfo>,
    /// A conta é admin do servidor (convidar, membros).
    pub admin: bool,
    /// Onboarding em tela cheia (primeira abertura de cada versão do onboarding).
    pub onboarding: Option<crate::views::onboarding::Onboarding>,
    /// Pulou o onboarding antes do teste dos encoders terminar: grava o nível recomendado
    /// quando o teste acabar.
    pub pending_preset: bool,
}

/// O que o launcher (splash) já carregou antes de abrir a janela.
pub struct Preloaded {
    pub config: Config,
    pub store: Store,
    /// Miniaturas da primeira tela.
    pub thumbs: Vec<(std::path::PathBuf, thumbs::Thumb)>,
    pub server_info: Option<app_cloud::ServerInfo>,
    /// Conta (nome, admin) quando o servidor respondeu a tempo.
    pub account: Option<(String, bool)>,
    /// O token salvo foi recusado (desconectar).
    pub unauthorized: bool,
}

impl Shell {
    pub fn new(
        inbox: Inbox,
        link: Option<String>,
        pre: Option<Preloaded>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("top.search")));
        cx.subscribe(&search, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let (config, store, pre) = match pre {
            Some(p) => {
                let Preloaded {
                    config,
                    store,
                    thumbs,
                    server_info,
                    account,
                    unauthorized,
                } = p;
                (
                    config,
                    store,
                    Some((thumbs, server_info, account, unauthorized)),
                )
            }
            None => (
                Config::load_or_create(&Config::default_path()).unwrap_or_default(),
                Store::open(),
                None,
            ),
        };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            screen: Screen::Library,
            focus,
            store,
            thumbs: Thumbs::default(),
            filter: Filter::All,
            sort: Sort::Newest,
            hovered: None,
            preview: None,
            search,
            config,
            status: None,
            clips_saved: None,
            copied: None,
            notice: None,
            starting_recorder: false,
            clip: None,
            audio: None,
            audio_tried: false,
            track_on: HashMap::new(),
            watermark_on: HashMap::new(),
            muted: false,
            fullscreen: false,
            token: secret::load(),
            job: None,
            queue: Vec::new(),
            export: None,
            selected: HashSet::new(),
            settings: None,
            account: None,
            pointer_at: Instant::now(),
            update: UpdateState::None,
            inbox,
            join: None,
            create: None,
            feedback: None,
            server_modal: None,
            server_info: None,
            admin: false,
            onboarding: None,
            pending_preset: false,
        };
        this.watch_updates(cx);
        let mut fetched = false;
        if let Some((thumbs, info, account, unauthorized)) = pre {
            for (path, t) in thumbs {
                this.thumbs.insert(path, t);
            }
            fetched = info.is_some() && (account.is_some() || this.token.is_none());
            this.server_info = info;
            if let Some((name, admin)) = account {
                this.account = Some(name);
                this.admin = admin;
            }
            if unauthorized {
                secret::delete();
                this.token = None;
            }
        }
        this.load_thumbs(cx);
        if !fetched {
            this.fetch_account(cx);
        }
        this.poll_recorder(window, cx);
        this.follow_look(window, cx);
        this.watch_inbox(window, cx);
        this.start_onboarding(window, cx);
        if let Some(l) = link {
            this.open_link(&l, cx);
        }
        this
    }

    /// Outra execução do app passou a vez: traz a janela e abre o link, se veio um.
    fn watch_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let alive = this.update_in(cx, |t, window, cx| {
                while let Some(link) = t.inbox.try_next() {
                    window.activate_window();
                    if let Some(l) = link {
                        t.open_link(&l, cx);
                    }
                }
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
    }

    /// Tema claro/escuro e transparência seguem o Windows sem reiniciar.
    fn follow_look(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.observe_window_appearance(window, |_, window, cx| theme::refresh(window, cx))
            .detach();
        cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                theme::refresh(window, cx);
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_secs(60))
                .await;
            let alive = this.update_in(cx, |_, window, cx| theme::refresh(window, cx));
            if alive.is_err() {
                break;
            }
        })
        .detach();
    }

    /// Pergunta o status ao gravador a cada segundo enquanto a janela existir.
    fn poll_recorder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| loop {
            let st = cx
                .background_executor()
                .spawn(async { recorder::status() })
                .await;
            if this.update(cx, |t, cx| t.on_status(st, cx)).is_err() {
                break;
            }
            cx.background_executor().timer(Duration::from_secs(1)).await;
        })
        .detach();
    }

    pub(crate) fn on_status(&mut self, st: Option<RecorderStatus>, cx: &mut Context<Self>) {
        let saved = st.as_ref().map(|s| s.clips_saved);
        // Clipe novo (ou o gravador reiniciou): relê o banco.
        if saved.is_some() && saved != self.clips_saved && self.clips_saved.is_some() {
            self.reload(cx);
        }
        if saved.is_some() {
            self.clips_saved = saved;
            self.starting_recorder = false;
        }
        self.onboarding_status(st.as_ref());
        if self.status != st {
            self.status = st;
            self.sync_pause();
            cx.notify();
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.store.reload();
        self.load_thumbs(cx);
        cx.notify();
    }

    /// Carrega em segundo plano as miniaturas que faltam.
    pub fn load_thumbs(&mut self, cx: &mut Context<Self>) {
        let missing = self.thumbs.missing(
            self.store
                .clips
                .iter()
                .filter_map(|c| c.thumb_path.as_deref()),
        );
        if missing.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            for path in missing {
                let p = path.clone();
                let img = cx
                    .background_executor()
                    .spawn(async move { thumbs::load(&p, thumbs::CARD_W) })
                    .await;
                let Some(img) = img else { continue };
                if this
                    .update(cx, |t, cx| {
                        t.thumbs.insert(path, img);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Mouse entrou/saiu de um card: depois de [`PREVIEW_DELAY`] parado em cima, o clipe
    /// toca mudo na miniatura (um player por vez, aberto fora da thread da UI).
    pub fn set_hover(&mut self, id: i64, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if on {
            self.hovered = Some(id);
            let Some(path) = self
                .store
                .index_of(id)
                .map(|i| self.store.clips[i].path.clone())
            else {
                return;
            };
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(PREVIEW_DELAY).await;
                let still = this.update(cx, |t, _| t.hovered == Some(id) && t.clip.is_none());
                if !matches!(still, Ok(true)) {
                    return;
                }
                let opened = cx
                    .background_executor()
                    .spawn(async move { Player::open(&path, HwPreference::Auto, (480, 270)) })
                    .await;
                let _ = this.update_in(cx, |t, window, cx| match opened {
                    Ok(p) if t.hovered == Some(id) && t.clip.is_none() => {
                        t.stop_preview(window);
                        t.preview = Some((id, VideoPlayer::from_player(p)));
                        cx.notify();
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("prévia do card: {e}"),
                });
            })
            .detach();
        } else if self.hovered == Some(id) {
            self.hovered = None;
            self.stop_preview(window);
        }
        cx.notify();
    }

    pub fn stop_preview(&mut self, window: &mut Window) {
        if let Some((_, p)) = self.preview.take() {
            p.release(window);
        }
    }

    pub fn toggle_favorite(&mut self, id: i64, cx: &mut Context<Self>) {
        if let Some(i) = self.store.index_of(id) {
            let on = !self.store.clips[i].favorite;
            self.store.set_favorite(id, on);
            cx.notify();
        }
    }

    /// Copia o link publicado e mostra "Link copiado" por 2 s.
    pub fn copy_link(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(url) = self
            .store
            .index_of(id)
            .and_then(|i| self.store.clips[i].published_url.clone())
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(url));
        self.copied = Some(id);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |t, cx| {
                if t.copied == Some(id) {
                    t.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Mostra um aviso curto no rodapé por alguns segundos.
    pub fn notify_user(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        self.notice = Some(text.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = this.update(cx, |t, cx| {
                if t.notice.as_ref() == Some(&text) {
                    t.notice = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn start_recorder(&mut self, cx: &mut Context<Self>) {
        match recorder::start() {
            Ok(()) => self.starting_recorder = true,
            Err(e) => self.notify_user(tr_args("recorder.start_error", &[("error", &e)]), cx),
        }
        cx.notify();
    }

    pub fn go(&mut self, screen: Screen, filter: Filter, cx: &mut Context<Self>) {
        self.drop_clip(cx);
        self.screen = screen;
        self.filter = filter;
        self.hovered = None;
        cx.notify();
    }

    pub fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }

    fn sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let lib = self.screen == Screen::Library;
        let clips = &self.store.clips;
        let total = clips.len() as u32;
        let fav = clips.iter().filter(|c| c.favorite).count() as u32;
        let publ = clips.iter().filter(|c| c.published_url.is_some()).count() as u32;
        let kills = clips.iter().filter(|c| c.kind == ClipKind::Kill).count() as u32;
        let games = data::games(clips);
        let used = self.store.total_bytes();
        let limit = self.config.disk_limit_bytes();

        let nav = |id: &'static str, icon: &'static str, label: SharedString, n: u32, f: Filter| {
            let active = lib && self.filter == f;
            NavItem::new(id, icon, label)
                .count(n)
                .active(active)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.go(Screen::Library, f.clone(), cx)),
                )
        };

        glass::glass_panel(R_PANEL)
            .flex()
            .flex_col()
            .flex_none()
            .w(px(232.))
            .h_full()
            .pt(px(16.))
            .px(px(10.))
            .pb(px(12.))
            .child(drag_area(
                "brand-drag",
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .px(px(8.))
                    .pb(px(16.))
                    // Símbolo colorido (o SVG da marca, rasterizado pelo GPUI) + wordmark minúsculo.
                    .child(img(BRAND_MARK).size(px(28.)).flex_none())
                    .child(
                        div()
                            .font_family(FONT_UI)
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tok::fg())
                            .child(brand::APP_NAME.to_lowercase()),
                    ),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(nav(
                        "nav-lib",
                        icon::GRID,
                        tr("nav.library"),
                        total,
                        Filter::All,
                    ))
                    .child(nav(
                        "nav-fav",
                        icon::STAR,
                        tr("nav.favorites"),
                        fav,
                        Filter::Favorites,
                    ))
                    .child(nav(
                        "nav-pub",
                        icon::LINK,
                        tr("nav.published"),
                        publ,
                        Filter::Published,
                    ))
                    .child(nav(
                        "nav-kills",
                        icon::GAMEPAD,
                        tr("nav.kills"),
                        kills,
                        Filter::Kills,
                    )),
            )
            .when(!games.is_empty(), |d| {
                d.child(
                    div()
                        .id("games")
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .child(
                            div()
                                .px(px(10.))
                                .pt(px(18.))
                                .pb(px(6.))
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(tok::fg_2())
                                .child(tr("nav.games")),
                        )
                        .children(games.into_iter().enumerate().map(|(i, (g, n))| {
                            let f = Filter::Game(g.clone());
                            let name: SharedString = g
                                .clone()
                                .map(Into::into)
                                .unwrap_or_else(|| tr("nav.screen"));
                            GameItem {
                                id: ElementId::NamedInteger("game".into(), i as u64),
                                name,
                                count: n,
                                active: lib && self.filter == f,
                            }
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.go(Screen::Library, f.clone(), cx),
                            ))
                        })),
                )
            })
            .child(div().flex_1().min_h(px(12.)))
            .children(
                self.update_card(cx)
                    .map(|c| div().mx(px(2.)).mb(px(8.)).child(c)),
            )
            .child(div().mx(px(2.)).mb(px(8.)).child(self.rec_card(cx)))
            .child(StorageMeter {
                label: tr("storage.disk"),
                value: if limit > 0 {
                    tr_args(
                        "storage.used_of",
                        &[
                            ("used", &format::size(used)),
                            ("total", &format::size(limit)),
                        ],
                    )
                } else {
                    format::size(used).into()
                },
                fraction: if limit > 0 {
                    used as f32 / limit as f32
                } else {
                    0.0
                },
            })
            .child(
                NavItem::new("nav-settings", icon::SETTINGS, tr("nav.settings"))
                    .active(self.screen == Screen::Settings)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.go(Screen::Settings, this.filter.clone(), cx)
                    })),
            )
    }

    /// Card do gravador na sidebar (estado, hotkey e "Iniciar" com ele fechado).
    fn rec_card(&mut self, cx: &mut Context<Self>) -> RecStatus {
        let shown = Shown::of(self.status.as_ref());
        let off = shown == Shown::Off;
        let hotkey = self
            .status
            .as_ref()
            .map(|s| s.hotkey.clone())
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| self.config.save_hotkey.clone());
        RecStatus {
            label: shown.label().into(),
            meta: if off {
                SharedString::default()
            } else {
                tr_args(
                    "top.save_hint",
                    &[
                        ("key", &hotkey),
                        ("secs", &self.config.buffer_secs.to_string()),
                    ],
                )
            },
            recording: shown.recording(),
            action: off.then(|| {
                Btn::new(
                    "start-rec",
                    BtnKind::Secondary,
                    if self.starting_recorder {
                        tr("recorder.starting")
                    } else {
                        tr("recorder.start")
                    },
                )
                .size(Size::Sm)
                .full_width()
                .disabled(self.starting_recorder)
                .on_click(cx.listener(|this, _, _, cx| this.start_recorder(cx)))
                .into_any_element()
            }),
        }
    }

    /// Título da tela e o resumo ao lado (só na biblioteca).
    fn heading(&self, cx: &App) -> (SharedString, Option<SharedString>) {
        match self.screen {
            Screen::Settings => (tr("nav.settings"), None),
            Screen::Library => (self.title_for_filter(), Some(self.library_summary(cx))),
        }
    }

    /// Barra do topo em pílulas: título, busca, ordem e os botões da janela. O título é
    /// área de arrasto da janela.
    fn topbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let sort = self.sort;
        let lib = self.screen == Screen::Library;
        let (title, sub) = self.heading(cx);
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .flex_none()
            .pl(px(14.))
            .pr(px(6.))
            .py(px(6.))
            .child(drag_area(
                "top-drag",
                // O GPUI não alinha por linha de base: as linhas têm a altura do texto (Inter:
                // ascender 0,969 + descender 0,242 em) e ficam alinhadas por baixo; o resumo sobe
                // a diferença dos descenders (0,242 × (26 − 14) ≈ 3 px).
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .h(Size::Md.h())
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_end()
                            .gap(px(10.))
                            .min_w_0()
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(26.))
                                    .line_height(px(32.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(tok::fg())
                                    .child(title),
                            )
                            .children(sub.map(|s| {
                                div()
                                    .min_w_0()
                                    .mb(px(3.))
                                    .truncate()
                                    .text_size(px(14.))
                                    .line_height(px(17.))
                                    .text_color(tok::fg_2())
                                    .child(s)
                            })),
                    ),
            ))
            .when(lib && !self.selected.is_empty(), |d| {
                let n = self.selected.len().to_string();
                d.child(
                    div()
                        .flex_none()
                        .text_size(Size::Md.text())
                        .text_color(tok::fg_2())
                        .child(tr_args("library.selected", &[("n", &n)])),
                )
                .child(
                    Btn::new(
                        "sel-publish",
                        BtnKind::Primary,
                        tr("library.publish_selected"),
                    )
                    .icon(icon::UPLOAD)
                    .on_click(cx.listener(|this, _, _, cx| this.publish_selected(cx))),
                )
                .child(
                    Btn::new("sel-clear", BtnKind::Ghost, tr("library.clear_selection"))
                        .on_click(cx.listener(|this, _, _, cx| this.clear_selection(cx))),
                )
            })
            .when(lib, |d| {
                d.child(SearchField::new(&self.search)).child(
                    glass::pill(theme::pal())
                        .id("sort")
                        .flex_none()
                        .gap(px(6.))
                        .text_size(Size::Md.text())
                        .text_color(tok::fg())
                        .cursor_pointer()
                        .hover(|s| s.bg(tok::surf_2()))
                        .child(tr(sort.key()))
                        .child(
                            svg()
                                .path(icon::CHEVRON_DOWN)
                                .size(Size::Sm.icon())
                                .text_color(tok::fg_2()),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sort = this.sort.next();
                            cx.notify();
                        })),
                )
            })
            .child(div().ml(px(8.)).child(WindowControls::new(theme::pal())))
    }

    fn footer_notice(&self) -> Option<Div> {
        let text = self.notice.clone().or_else(|| {
            self.store
                .error
                .as_ref()
                .map(|e| tr_args("library.db_error", &[("error", e)]))
        })?;
        Some(
            div()
                .flex_none()
                .mx(px(6.))
                .px(px(14.))
                .py(px(10.))
                .rounded(R_SM)
                .bg(tok::surf())
                .border_1()
                .border_color(tok::edge())
                .font_family(FONT_UI)
                .text_size(px(13.))
                .text_color(tok::fg_2())
                .child(text),
        )
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.tick_clip(window) {
            window.request_animation_frame();
        }
        if self.clip.is_some() {
            self.stop_preview(window);
        }
        if let Some((_, p)) = self.preview.as_mut() {
            if p.tick(window) {
                window.request_animation_frame();
            }
        }
        let root = div()
            .id("shell")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if this.onboarding_key(e, cx) || this.clip_key(e, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .flex()
            .flex_row()
            .bg(tok::bg())
            .when(!self.fullscreen, |d| d.gap(INSET).p(INSET))
            .font_family(FONT_UI)
            .text_color(tok::fg())
            .text_size(px(14.));
        if self.clip.is_some() && self.fullscreen {
            return root.child(self.clip_fullscreen(window, cx));
        }

        let notice = self.footer_notice();
        // Onboarding: tela cheia no lugar da biblioteca (os assistentes do servidor abrem por cima).
        if self.onboarding.is_some() && self.clip.is_none() {
            let screen = self.onboarding_screen(window, cx);
            let join = self.join_modal(window, cx);
            let create = self.create_modal(window, cx);
            return root
                .relative()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .gap(px(6.))
                        .child(div().flex().flex_1().min_h_0().child(screen))
                        .children(notice),
                )
                .children(join)
                .children(create);
        }
        // Clipe aberto: tela própria, sem sidebar nem barra da biblioteca (a grade nem é
        // desenhada; o player redesenha a cada quadro).
        if self.clip.is_some() {
            let modal = self.clip_modal(cx);
            let join = self.join_modal(window, cx);
            let create = self.create_modal(window, cx);
            let feedback = self.feedback_modal(window, cx);
            return root
                .relative()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .gap(px(6.))
                        .child(self.clip_screen(window, cx))
                        .children(notice),
                )
                .children(modal)
                .children(join)
                .children(create)
                .children(feedback);
        }

        let list = match self.screen {
            Screen::Library => self.library(window, cx).into_any_element(),
            Screen::Settings => self.settings_screen(window, cx),
        };
        let content = div()
            .id("content")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(list);

        let server_modal = self.server_modal_view(cx);
        let join = self.join_modal(window, cx);
        let create = self.create_modal(window, cx);
        let feedback = self.feedback_modal(window, cx);
        root.relative()
            .child(self.sidebar(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .gap(px(6.))
                    .child(self.topbar(cx))
                    .child(content)
                    .children(notice),
            )
            .children(server_modal)
            .children(join)
            .children(create)
            .children(feedback)
    }
}
