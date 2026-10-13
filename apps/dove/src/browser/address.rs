//! The browser's address field: a single-line gpui-base input with the small
//! API the ported browser chrome expects (`text`, `set_text`, edit events).
//! `set_text` works without a window: the value lands on the next render.

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::theme::Theme;
use crate::ui;

pub enum AddressEvent {
    /// The user changed the text.
    Edited,
}

pub struct AddressInput {
    state: Entity<InputState>,
    pending: Option<String>,
    _sub: Subscription,
}

impl EventEmitter<AddressEvent> for AddressInput {}

impl Focusable for AddressInput {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.state.read(cx).focus_handle(cx)
    }
}

impl AddressInput {
    pub fn new(placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let state = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder(placeholder);
            s.set_editor_style(style);
            s
        });
        let _sub = cx.subscribe(&state, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.emit(AddressEvent::Edited);
            }
        });
        Self { state, pending: None, _sub }
    }

    pub fn text(&self, cx: &gpui::App) -> String {
        match &self.pending {
            Some(p) => p.clone(),
            None => self.state.read(cx).value().to_string(),
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.pending = Some(text.into());
        cx.notify();
    }

    pub fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush(window, cx);
        self.state.update(cx, |s, cx| s.select_all(window, cx));
    }

    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.pending.take() {
            self.state.update(cx, |s, cx| s.set_value(text, window, cx));
        }
    }
}

impl Render for AddressInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.flush(window, cx);
        div().w_full().child(Input::new(&self.state))
    }
}
