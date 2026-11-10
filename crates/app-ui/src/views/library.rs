use app_core::library::ClipKind;
use gpui_kit::*;

use crate::app::Shell;
use crate::components::{BadgeKind, ClipCard, DayDivider};
use crate::data::{self, Filter};
use crate::format;
use crate::publisher::Phase;
use crate::theme::{self, tok};
use crate::{tr, tr_args};

/// Largura mínima de um card; a grade calcula as colunas a partir dela.
pub const CARD_MIN_W: f32 = 232.;
pub const GAP_X: f32 = 16.;
pub const GAP_Y: f32 = 18.;
/// Largura ocupada fora da grade (margens + menu lateral + respiro lateral + rolagem).
const CHROME_W: f32 = 10. + 232. + 10. + 28. + 10. + 8.;

/// Quantas colunas cabem em `width`.
pub fn columns(width: f32) -> u16 {
    (((width + GAP_X) / (CARD_MIN_W + GAP_X)).floor() as u16).max(1)
}

impl Shell {
    /// "12 clipes · 9 min" do que está visível.
    pub fn library_summary(&self, cx: &App) -> SharedString {
        let order = data::visible(&self.store.clips, &self.filter, &self.query(cx), self.sort);
        let total_ms: i64 = order.iter().map(|&i| self.store.clips[i].duration_ms).sum();
        let minutes = ((total_ms as f64) / 60_000.0).round() as i64;
        tr_args(
            if order.len() == 1 {
                "library.summary_one"
            } else {
                "library.summary"
            },
            &[
                ("count", &order.len().to_string()),
                ("minutes", &minutes.to_string()),
            ],
        )
    }

    pub fn title_for_filter(&self) -> SharedString {
        match &self.filter {
            Filter::All => tr("library.title"),
            Filter::Favorites => tr("nav.favorites"),
            Filter::Published => tr("nav.published"),
            Filter::Kills => tr("nav.kills"),
            Filter::Game(Some(g)) => g.clone().into(),
            Filter::Game(None) => tr("nav.screen"),
        }
    }

    pub fn library(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cols = columns(f32::from(window.viewport_size().width) - CHROME_W);
        let query = self.query(cx);
        let order = data::visible(&self.store.clips, &self.filter, &query, self.sort);

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .px(px(14.))
            .pt(px(8.))
            .pb(px(20.));

        if order.is_empty() {
            let (title, hint) = if self.store.clips.is_empty() {
                (
                    tr("library.empty_title"),
                    tr_args("library.empty_hint", &[("key", &self.config.save_hotkey)]),
                )
            } else {
                (tr("library.no_match"), tr("library.no_match_hint"))
            };
            return body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .py(px(48.))
                    .max_w(px(460.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(19.))
                            .text_color(tok::muted())
                            .child(hint),
                    ),
            );
        }

        let grid = |items: Vec<usize>, this: &Self, cx: &mut Context<Self>| {
            div()
                .grid()
                .grid_cols(cols)
                .gap_x(px(GAP_X))
                .gap_y(px(GAP_Y))
                .children(items.into_iter().map(|i| this.card(i, cx)))
        };

        if self.sort.by_date() {
            let today = format::local(now_ms()).date_naive();
            for (day, items) in data::group_by_day(&self.store.clips, &order) {
                body = body
                    .child(DayDivider(format::day_label(day, today).into()))
                    .child(grid(items, self, cx));
            }
        } else {
            body = body.child(grid(order, self, cx));
        }
        body
    }

    fn toggle_selected(&mut self, id: i64, cx: &mut Context<Self>) {
        if !self.selected.remove(&id) {
            self.selected.insert(id);
        }
        cx.notify();
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected.clear();
        cx.notify();
    }

    /// "Enviando 42%", "Pausado" ou "Na fila" no card.
    fn upload_badge(&self, id: i64) -> Option<SharedString> {
        if let Some(j) = self
            .job
            .as_ref()
            .filter(|j| j.clip_id == id && j.phase.running())
        {
            if j.paused {
                return Some(tr("publish.paused_badge"));
            }
            let frac = match j.phase {
                Phase::Export(f) => f * 0.5,
                Phase::Upload { sent, total } => 0.5 + 0.5 * sent as f32 / total.max(1) as f32,
                Phase::Finishing => 1.0,
                _ => 0.0,
            };
            return Some(format!("{} {:.0}%", tr("publish.sending_badge"), frac * 100.0).into());
        }
        self.queue_pos(id).map(|_| tr("publish.queue_badge"))
    }

    pub fn card(&self, i: usize, cx: &mut Context<Self>) -> ClipCard {
        let c = &self.store.clips[i];
        let id = c.id;
        let game: SharedString = c
            .game
            .clone()
            .map(Into::into)
            .unwrap_or_else(|| tr("nav.screen"));
        let meta = format!(
            "{} · {} · {}",
            game,
            format::size(c.size_bytes),
            format::relative(c.created_ms, now_ms())
        );
        let card = ClipCard::new(
            ("clip", id as u64),
            c.thumb_path.as_deref().and_then(|p| self.thumbs.get(p)),
            data::title_of(c, &tr("clip.untitled")),
            game,
            meta,
            format::short(c.duration_ms as f32 / 1000.0),
        )
        .blur(c.thumb_path.as_deref().and_then(|p| self.thumbs.blur(p)))
        .favorite(c.favorite)
        .hovered(self.hovered == Some(id))
        .selected(self.selected.contains(&id))
        .selecting(!self.selected.is_empty())
        .on_select(cx.listener(move |this, _, _, cx| this.toggle_selected(id, cx)))
        .on_hover(cx.listener(move |this, on: &bool, w, cx| this.set_hover(id, *on, w, cx)))
        .on_favorite(cx.listener(move |this, _, _, cx| this.toggle_favorite(id, cx)))
        .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
            // Ctrl+clique seleciona; com algo selecionado, o clique só marca/desmarca.
            if ev.modifiers().control || !this.selected.is_empty() {
                this.toggle_selected(id, cx);
            } else {
                this.open_clip(id, window, cx);
            }
        }));
        let card = match self.preview.as_ref().filter(|(pid, _)| *pid == id) {
            Some((_, p)) => {
                let t = p.position_ms() as f32 / p.duration_ms().max(1) as f32;
                card.preview(p.surface_rounded(theme::R), t)
            }
            None => card,
        };
        // Selo "Kill" (ou "x2", "x3"... num combo) no canto de baixo da miniatura.
        let card = match (c.kind, c.kills) {
            (ClipKind::Kill, Some(n)) if n > 1 => card.kill(format!("x{n}")),
            (ClipKind::Kill, _) => card.kill(tr("card.kill")),
            _ => card,
        };
        if let Some(label) = self.upload_badge(id) {
            return card.badge(BadgeKind::Upload, label);
        }
        if c.published_url.is_some() {
            let done = self.copied == Some(id);
            card.badge(BadgeKind::Published, tr("card.published"))
                .link(
                    tr(if done {
                        "card.copied"
                    } else {
                        "card.copy_link"
                    }),
                    done,
                )
                .on_copy(cx.listener(move |this, _, _, cx| this.copy_link(id, cx)))
        } else {
            card
        }
    }
}

/// Agora em ms desde a época.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{columns, CARD_MIN_W, GAP_X};

    #[test]
    fn colunas_da_grade() {
        assert_eq!(columns(100.), 1);
        assert_eq!(columns(CARD_MIN_W), 1);
        assert_eq!(columns(CARD_MIN_W * 2. + GAP_X), 2);
        assert_eq!(columns(1000.), 4);
    }
}
