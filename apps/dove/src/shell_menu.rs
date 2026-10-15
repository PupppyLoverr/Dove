//! The menus the shell draws itself: the sidebar's More and options menus,
//! a thread's context menu and the tab overflow. One grammar for all of
//! them: kit menu rows, a highlight that follows the mouse and the keys
//! (↑/↓, Home/End, Ctrl+N/P), Enter or Space to activate, Esc to close, the
//! 140 ms entrance and a 100 ms exit that replays the closed menu as an
//! inert ghost.

use std::rc::Rc;
use std::time::Instant;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Div, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::kit::popover::{self, MenuKey};
use crate::motion;
use crate::shell::{ChatMenu, Shell};
use crate::theme::Theme;

/// What a row does when picked.
pub type Act = Rc<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>;

pub fn act(f: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static) -> Act {
    Rc::new(f)
}

/// One entry of a shell menu.
pub enum Item {
    Row {
        id: SharedString,
        glyph: Option<&'static str>,
        label: SharedString,
        danger: bool,
        /// Sub-items (a disclosed submenu) sit under their parent's label.
        indent: bool,
        /// Drawn at reduced opacity (an option that is off).
        muted: bool,
        /// Picking it keeps the menu open (disclosures, two-step confirms).
        keep_open: bool,
        trailing: Option<AnyElement>,
        act: Act,
    },
    Separator,
}

impl Item {
    pub fn row(
        id: impl Into<SharedString>,
        glyph: Option<&'static str>,
        label: impl Into<SharedString>,
        act: Act,
    ) -> Self {
        Item::Row {
            id: id.into(),
            glyph,
            label: label.into(),
            danger: false,
            indent: false,
            muted: false,
            keep_open: false,
            trailing: None,
            act,
        }
    }

    fn set(mut self, f: impl FnOnce(&mut bool, &mut bool, &mut bool, &mut bool)) -> Self {
        if let Item::Row { danger, indent, muted, keep_open, .. } = &mut self {
            f(danger, indent, muted, keep_open);
        }
        self
    }

    pub fn danger(self) -> Self {
        self.set(|d, _, _, _| *d = true)
    }

    pub fn indent(self) -> Self {
        self.set(|_, i, _, _| *i = true)
    }

    pub fn muted(self, on: bool) -> Self {
        self.set(|_, _, m, _| *m = on)
    }

    pub fn keep_open(self) -> Self {
        self.set(|_, _, _, k| *k = true)
    }

    pub fn trailing(mut self, el: impl IntoElement) -> Self {
        if let Item::Row { trailing, .. } = &mut self {
            *trailing = Some(el.into_any_element());
        }
        self
    }
}

/// Which menu a closing ghost redraws.
#[derive(Clone)]
pub enum Which {
    More,
    Options,
    Overflow,
    Context(ChatMenu),
}

/// Keyboard and exit state shared by the shell's menus (one is open at a
/// time).
pub struct Menus {
    /// The highlighted row of the open menu.
    pub hi: Option<usize>,
    pub focus: FocusHandle,
    /// Focus to hand back when the menu closes.
    restore: Option<FocusHandle>,
    closing: Option<(Which, Instant)>,
}

impl Menus {
    pub fn new(cx: &mut App) -> Self {
        Self { hi: None, focus: cx.focus_handle(), restore: None, closing: None }
    }

    fn exit_of(&self, pick: impl Fn(&Which) -> bool) -> Option<f32> {
        let (which, at) = self.closing.as_ref()?;
        if !pick(which) {
            return None;
        }
        let now = Instant::now();
        (now.saturating_duration_since(*at) < motion::MENU_OUT.total()).then(|| motion::MENU_OUT.since(*at, now))
    }

    /// Exit progress of the More menu's ghost.
    pub fn more_exit(&self) -> Option<f32> {
        self.exit_of(|w| matches!(w, Which::More))
    }

    pub fn options_exit(&self) -> Option<f32> {
        self.exit_of(|w| matches!(w, Which::Options))
    }

    pub fn overflow_exit(&self) -> Option<f32> {
        self.exit_of(|w| matches!(w, Which::Overflow))
    }

    /// The context menu's ghost: what it showed, and its exit progress.
    pub fn context_exit(&self) -> Option<(ChatMenu, f32)> {
        let p = self.exit_of(|w| matches!(w, Which::Context(_)))?;
        match &self.closing {
            Some((Which::Context(m), _)) => Some((m.clone(), p)),
            _ => None,
        }
    }
}

impl Shell {
    /// A menu just opened: reset the highlight and take keyboard focus.
    pub fn menu_opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menus.hi = None;
        self.menus.closing = None;
        if !self.menus.focus.is_focused(window) {
            self.menus.restore = window.focused(cx);
        }
        window.focus(&self.menus.focus, cx);
    }

    /// Close whichever shell menu is open, leaving its exit ghost. True when
    /// one was open.
    pub fn close_menus(&mut self) -> bool {
        let which = if self.more_open {
            Some(Which::More)
        } else if self.sidebar_options_open {
            Some(Which::Options)
        } else if self.tab_overflow_open {
            Some(Which::Overflow)
        } else {
            self.menu.clone().map(Which::Context)
        };
        self.more_open = false;
        self.sidebar_options_open = false;
        self.tab_overflow_open = false;
        self.menu = None;
        self.menus.hi = None;
        let Some(which) = which else { return false };
        if !motion::reduced_now() {
            self.menus.closing = Some((which, Instant::now()));
        }
        true
    }

    /// Once per shell frame: retire a finished exit, keep frames coming
    /// while one runs, and hand focus back once no menu holds it.
    pub fn tick_menus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, at)) = &self.menus.closing {
            if Instant::now().saturating_duration_since(*at) >= motion::MENU_OUT.total() {
                self.menus.closing = None;
            } else {
                window.request_animation_frame();
            }
        }
        let open = self.more_open || self.sidebar_options_open || self.tab_overflow_open || self.menu.is_some();
        if !open && self.menus.focus.is_focused(window) {
            match self.menus.restore.take() {
                Some(prev) => window.focus(&prev, cx),
                None => window.focus(&self.focus, cx),
            }
        }
    }

    /// The rows of a shell menu. `live` is false for an exit ghost, which
    /// draws the same rows with no listeners.
    pub fn menu_list(&self, items: Vec<Item>, width: f32, live: bool, t: &Theme, cx: &mut Context<Self>) -> Div {
        let acts: Vec<(Act, bool)> = items
            .iter()
            .filter_map(|i| match i {
                Item::Row { act, keep_open, .. } => Some((act.clone(), *keep_open)),
                Item::Separator => None,
            })
            .collect();
        let count = acts.len();
        let hi = if live { self.menus.hi } else { None };
        let mut body = div().w(px(width)).flex().flex_col().gap(px(popover::MENU_GAP));
        if live {
            let acts = acts.clone();
            body = body.track_focus(&self.menus.focus).on_key_down(cx.listener(
                move |this, e: &KeyDownEvent, window, cx| {
                    let key = popover::classify(&e.keystroke.key, e.keystroke.modifiers.control);
                    match key {
                        MenuKey::Other => return,
                        MenuKey::Close => {
                            this.close_menus();
                        }
                        key => {
                            let mut hi = this.menus.hi;
                            match popover::apply_key(key, &mut hi, count).and_then(|i| acts.get(i).cloned()) {
                                Some((act, keep)) => {
                                    if !keep {
                                        this.close_menus();
                                    }
                                    act(this, window, cx);
                                }
                                None => this.menus.hi = hi,
                            }
                        }
                    }
                    cx.stop_propagation();
                    cx.notify();
                },
            ));
        }
        let mut n = 0usize;
        for item in items {
            match item {
                Item::Separator => body = body.child(popover::menu_separator(t)),
                Item::Row { id, glyph, label, danger, indent, muted, keep_open, trailing, act } => {
                    let i = n;
                    n += 1;
                    let row = popover::menu_item(t, id, glyph, label, danger, hi == Some(i), false)
                        .when(indent, |d| d.pl(px(33.0)).text_color(t.text_muted))
                        .when(muted, |d| d.opacity(0.75))
                        .children(trailing);
                    let row = if live {
                        row.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered && this.menus.hi != Some(i) {
                                this.menus.hi = Some(i);
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if !keep_open {
                                this.close_menus();
                            }
                            act(this, window, cx);
                            cx.notify();
                        }))
                    } else {
                        row
                    };
                    body = body.child(row);
                }
            }
        }
        body
    }

    /// The menu card: the entrance while open, the exit while closing.
    pub fn menu_surface(&self, t: &Theme, body: impl IntoElement, exit: Option<f32>) -> AnyElement {
        match exit {
            None => crate::ui::menu_card(t, body).into_any_element(),
            Some(p) => motion::menu_out(
                "menu-card-out",
                p,
                div().child(crate::ui::frosted(popover::CARD_RADIUS, 16.0, popover::card(t).child(body))),
            )
            .into_any_element(),
        }
    }
}
