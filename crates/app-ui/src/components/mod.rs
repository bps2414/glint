//! Componentes base do design system (Fase 1; vidro na Fase 5).
//!
//! São elementos sem estado (`RenderOnce`): o estado mora na view e chega por
//! parâmetro, os eventos voltam por callbacks. Todos usam só os tokens de
//! [`crate::theme`].

mod button;
mod card;
mod controls;
pub mod glass;
pub mod modal;
mod nav;
mod status;
mod timeline;
pub mod window_controls;

pub use button::{Btn, BtnKind, IconBtn, IconKind};
pub use card::ClipCard;
pub use controls::{field_label, input_box, text_field, Pill, SearchField, Segmented, Toggle};
pub use nav::{game_mark, GameItem, NavItem, StorageMeter};
pub use status::{Avatar, Badge, BadgeKind, DayDivider, DurationTag, Kbd, LinkDone, RecStatus};
pub use timeline::{Grab, Timeline, TimelineState, PAD_X as TIMELINE_PAD};

use std::rc::Rc;

use gpui_kit::{App, ClickEvent, Window};

/// Callback de evento compartilhável (mesma forma de `cx.listener`).
pub type Handler<E> = Rc<dyn Fn(&E, &mut Window, &mut App)>;

/// Callback de clique.
pub type OnClick = Handler<ClickEvent>;
