//! Floating surfaces: the popup lifecycle with an exit animation, the shared
//! menu card, rows, headings and separators, and the keyboard model every
//! menu follows. Adapted from Zeron's `popover.rs` (MIT).

use std::time::Instant;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::assets::icon;
use crate::design::ty;
use crate::motion;
use crate::theme::Theme;

/// Card corner radius; rows inside are concentric with it.
pub const CARD_RADIUS: f32 = 12.0;
/// The card's inner padding.
pub const CARD_INSET: f32 = 4.0;
/// Gap between rows.
pub const MENU_GAP: f32 = 2.0;
/// Row corners: the card's radius less its border and inset.
pub const MENU_ITEM_RADIUS: f32 = CARD_RADIUS - 1.0 - CARD_INSET;

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// A popup with an exit phase. GPUI unmounts an element the frame its state
/// drops, so a closing animation needs the state held while it plays:
/// `open` → `begin_close` (still mounted, fading out, no longer interactive)
/// → [`reap_popup`] drops it once [`motion::MENU_OUT`] has run.
pub struct Popup<T> {
    inner: Option<(T, Option<Instant>)>,
    pressed_while_open: bool,
}

impl<T> Default for Popup<T> {
    fn default() -> Self {
        Self { inner: None, pressed_while_open: false }
    }
}

impl<T> Popup<T> {
    pub fn open(&mut self, value: T) {
        self.inner = Some((value, None));
    }

    /// Open and interactive (a closing popup reads as closed).
    pub fn is_open(&self) -> bool {
        matches!(self.inner, Some((_, None)))
    }

    pub fn is_mounted(&self) -> bool {
        self.inner.is_some()
    }

    pub fn closing_since(&self) -> Option<Instant> {
        match &self.inner {
            Some((_, Some(since))) => Some(*since),
            _ => None,
        }
    }

    /// The state while mounted (open or fading out): for rendering.
    pub fn get(&self) -> Option<&T> {
        self.inner.as_ref().map(|(v, _)| v)
    }

    /// The state only while open: for event handlers.
    pub fn as_open(&self) -> Option<&T> {
        match &self.inner {
            Some((v, None)) => Some(v),
            _ => None,
        }
    }

    pub fn open_mut(&mut self) -> Option<&mut T> {
        match &mut self.inner {
            Some((v, None)) => Some(v),
            _ => None,
        }
    }

    /// Start the exit phase; `true` when this call started it.
    pub fn begin_close(&mut self) -> bool {
        match &mut self.inner {
            Some((_, closing @ None)) => {
                *closing = Some(Instant::now());
                true
            }
            _ => false,
        }
    }

    /// Close at once, without the exit animation.
    pub fn close_now(&mut self) {
        self.inner = None;
    }

    /// From the trigger's mouse-down: whether the popup is mounted, so the
    /// click that follows (after the card's outside-press closed it) does
    /// not reopen it.
    pub fn note_trigger_press(&mut self) {
        self.pressed_while_open = self.inner.is_some();
    }

    pub fn take_press_was_open(&mut self) -> bool {
        std::mem::take(&mut self.pressed_while_open)
    }

    fn finish_close(&mut self) {
        if let Some((_, Some(since))) = &self.inner
            && since.elapsed() >= motion::MENU_OUT.total()
        {
            self.inner = None;
        }
    }
}

/// After the exit animation, drop the popup's state and repaint. `popup`
/// re-borrows the field from the view that owns it.
pub fn reap_popup<V: 'static, T: 'static>(cx: &mut Context<V>, popup: impl Fn(&mut V) -> &mut Popup<T> + 'static) {
    cx.spawn(async move |view, cx| {
        cx.background_executor()
            .timer(motion::MENU_OUT.total().saturating_add(std::time::Duration::from_millis(20)))
            .await;
        let _ = view.update(cx, |view, cx| {
            popup(view).finish_close();
            cx.notify();
        });
    })
    .detach();
}

/// Close `popup` with its exit animation and schedule the reap.
pub fn close_popup<V: 'static, T: 'static>(
    view: &mut V,
    popup: impl Fn(&mut V) -> &mut Popup<T> + 'static,
    cx: &mut Context<V>,
) {
    if popup(view).begin_close() {
        reap_popup(cx, popup);
    }
    cx.notify();
}

/// Wrap a menu's card in its entrance (or, while closing, its exit).
pub fn animate(
    id: impl Into<ElementId>,
    closing: Option<Instant>,
    card: impl IntoElement + gpui::Styled + 'static,
) -> AnyElement {
    match closing {
        None => motion::menu_in(id, card).into_any_element(),
        Some(since) => {
            let t = motion::MENU_OUT.since(since, Instant::now());
            motion::menu_out(id, t, card).into_any_element()
        }
    }
}

// ---------------------------------------------------------------------------
// Keyboard model
// ---------------------------------------------------------------------------

/// Step the highlighted row, wrapping; entering from nothing lands on the
/// edge matching the direction.
pub fn menu_step(active: Option<usize>, count: usize, delta: isize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let n = count as isize;
    let next = match active {
        None if delta >= 0 => 0,
        None => n - 1,
        Some(at) => (at as isize + delta).rem_euclid(n),
    };
    Some(next as usize)
}

/// What a key press means to an open menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Home,
    End,
    Activate,
    Close,
    Other,
}

pub fn classify(key: &str, ctrl: bool) -> MenuKey {
    match key {
        "up" => MenuKey::Up,
        "down" => MenuKey::Down,
        "p" if ctrl => MenuKey::Up,
        "n" if ctrl => MenuKey::Down,
        "home" => MenuKey::Home,
        "end" => MenuKey::End,
        "enter" | "space" => MenuKey::Activate,
        "escape" => MenuKey::Close,
        _ => MenuKey::Other,
    }
}

/// Apply a menu key to the highlight. Returns the row to activate, if any.
pub fn apply_key(key: MenuKey, active: &mut Option<usize>, count: usize) -> Option<usize> {
    match key {
        MenuKey::Up => *active = menu_step(*active, count, -1),
        MenuKey::Down => *active = menu_step(*active, count, 1),
        MenuKey::Home => *active = (count > 0).then_some(0),
        MenuKey::End => *active = count.checked_sub(1),
        MenuKey::Activate => return *active,
        MenuKey::Close | MenuKey::Other => {}
    }
    None
}

/// `0` prefix match, `1` substring, `None` no match (case-insensitive).
pub fn match_rank(query: &str, label: &str) -> Option<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(1);
    }
    let label = label.to_lowercase();
    if label.starts_with(&query) {
        Some(0)
    } else if label.contains(&query) {
        Some(1)
    } else {
        None
    }
}

/// Indices of `labels` matching `query`: prefix matches first, stable.
pub fn filter_indices<S: AsRef<str>>(query: &str, labels: &[S]) -> Vec<usize> {
    let mut ranked: Vec<(usize, usize)> =
        labels.iter().enumerate().filter_map(|(i, l)| match_rank(query, l.as_ref()).map(|r| (r, i))).collect();
    ranked.sort_unstable();
    ranked.into_iter().map(|(_, i)| i).collect()
}

// ---------------------------------------------------------------------------
// Elements
// ---------------------------------------------------------------------------

/// The fill of floating surfaces: legible over any backdrop on glass, the
/// overlay tone when opaque.
pub fn surface_bg(t: &Theme) -> Hsla {
    t.glass_overlay()
}

/// The floating card: hairline border, 12 px corners, 4 px inset, a soft
/// shadow, and a backdrop blur on glass.
pub fn card(t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(MENU_GAP))
        .p(px(CARD_INSET))
        .rounded(px(CARD_RADIUS))
        .bg(surface_bg(t))
        .border_1()
        .border_color(t.border_strong)
        .shadow(vec![gpui::BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, if t.appearance.is_dark() { 0.38 } else { 0.14 }),
            offset: gpui::point(px(0.0), px(10.0)),
            blur_radius: px(30.0),
            spread_radius: px(0.0),
            inset: false,
        }])
        .text_size(ty::BODY)
        .text_color(t.text)
}

/// [`card`] mounted in its frosted layer.
pub fn frosted_card(t: &Theme, body: impl IntoElement) -> AnyElement {
    crate::ui::frosted(CARD_RADIUS, 16.0, card(t).child(body)).into_any_element()
}

/// The selected / keyboard-highlighted row wash.
pub fn row_active_bg(t: &Theme) -> Hsla {
    t.wash(if t.appearance.is_dark() { 0.10 } else { 0.11 })
}

/// One menu row: 13 px label, 8 × 6 padding, concentric corners, and a hover
/// wash that fades in over 150 ms. `id` doubles as the fade key, so it must
/// be unique and stable. Fades need their view redrawn every frame, so use
/// this in menus the shell draws; inside a cached view (a thread, the side
/// panel) use [`menu_row_instant`].
pub fn menu_row(t: &Theme, id: impl Into<SharedString>, highlighted: bool) -> gpui::Stateful<Div> {
    let id: SharedString = id.into();
    let key = id.clone();
    let rest = if highlighted { row_active_bg(t) } else { t.wash(0.0) };
    row_frame(id)
        .text_color(if highlighted { t.text } else { motion::hover_blend(&key, t.text.opacity(0.9), t.text) })
        .bg(motion::hover_blend(&key, rest, row_active_bg(t)))
        .on_hover(motion::hover_listener(key))
}

/// [`menu_row`] whose hover wash applies at once: for menus inside views the
/// shell caches between frames.
pub fn menu_row_instant(t: &Theme, id: impl Into<SharedString>, highlighted: bool) -> gpui::Stateful<Div> {
    let hover = row_active_bg(t);
    let text = t.text;
    row_frame(id.into())
        .text_color(if highlighted { t.text } else { t.text.opacity(0.9) })
        .when(highlighted, |d| d.bg(row_active_bg(t)))
        .hover(move |s| s.bg(hover).text_color(text))
}

fn row_frame(id: SharedString) -> gpui::Stateful<Div> {
    div()
        .id(ElementId::Name(id))
        .role(gpui::Role::MenuItem)
        .min_h(px(30.0))
        .px(px(8.0))
        .py(px(5.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .rounded(px(MENU_ITEM_RADIUS))
        .text_size(ty::BODY)
        .cursor_pointer()
}

/// A menu row with a leading glyph and a label (`danger` tints both).
pub fn menu_item(
    t: &Theme,
    id: impl Into<SharedString>,
    glyph: Option<&'static str>,
    label: impl Into<SharedString>,
    danger: bool,
    highlighted: bool,
    fade: bool,
) -> gpui::Stateful<Div> {
    let label: SharedString = label.into();
    let row = if fade { menu_row(t, id, highlighted) } else { menu_row_instant(t, id, highlighted) };
    row.aria_label(label.clone())
        .when(danger, |d| d.text_color(t.danger))
        .when_some(glyph, |d, g| {
            d.child(icon(g).size(px(15.0)).flex_none().text_color(if danger { t.danger } else { t.text_muted }))
        })
        .child(div().flex_1().min_w_0().truncate().child(label))
}

/// A small heading over a group of rows.
pub fn menu_heading(t: &Theme, label: impl Into<SharedString>) -> Div {
    div()
        .px(px(8.0))
        .pt(px(6.0))
        .pb(px(2.0))
        .text_size(ty::CAPTION)
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(t.text_faint)
        .child(label.into())
}

/// A hairline between groups, running border to border.
pub fn menu_separator(t: &Theme) -> Div {
    div().h(px(1.0)).mx(px(-CARD_INSET)).my(px(MENU_GAP)).bg(t.border)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_wraps_and_enters_at_edges() {
        assert_eq!(menu_step(None, 3, 1), Some(0));
        assert_eq!(menu_step(None, 3, -1), Some(2));
        assert_eq!(menu_step(Some(2), 3, 1), Some(0));
        assert_eq!(menu_step(Some(0), 3, -1), Some(2));
        assert_eq!(menu_step(None, 0, 1), None);
    }

    #[test]
    fn keys_move_and_activate() {
        let mut active = None;
        assert_eq!(apply_key(MenuKey::Down, &mut active, 4), None);
        assert_eq!(active, Some(0));
        apply_key(MenuKey::End, &mut active, 4);
        assert_eq!(active, Some(3));
        assert_eq!(apply_key(MenuKey::Activate, &mut active, 4), Some(3));
        assert_eq!(classify("n", true), MenuKey::Down);
        assert_eq!(classify("escape", false), MenuKey::Close);
    }

    #[test]
    fn filter_ranks_prefix_first() {
        let labels = ["Appearance", "Reduce motion", "Archived"];
        // "Archived" starts with "ar"; "Appearance" only contains it.
        assert_eq!(filter_indices("ar", &labels), vec![2, 0]);
        assert_eq!(filter_indices("arc", &labels), vec![2]);
        assert_eq!(filter_indices("a", &labels), vec![0, 2]);
        assert_eq!(filter_indices("", &labels), vec![0, 1, 2]);
    }

    #[test]
    fn popup_lifecycle() {
        let mut p: Popup<u8> = Popup::default();
        p.open(1);
        assert!(p.is_open());
        assert!(p.begin_close());
        assert!(!p.is_open() && p.is_mounted());
        assert!(!p.begin_close());
        p.close_now();
        assert!(!p.is_mounted());
    }
}
