//! First Light: the first run's arrival and its hand-off to setup.
//!
//! The intro is an overlay over the whole display (`covers_display`, always
//! on top, transparent, clicks passing through until it asks for them). Its
//! script draws a dark sky fading in and Keplorayx's mark drawing its orbit
//! as the wordmark resolves, capped at 60 frames a second.
//!
//! The hand-off is four beats, so there are no seams:
//!   - `prepare { x, y, w, h }`: setup's window opens hidden at that rect
//!     (points from the overlay's top left), its script built and its first
//!     frame drawn meanwhile. Its corner radius comes back as
//!     `intro.radius`, so the orbit closes onto exactly that shape.
//!   - `reveal(at)`: setup's window shows, under the overlay's still-opaque
//!     sky; `at` (the overlay's clock) becomes setup's `onboarding.epoch`, so
//!     anything both draw moves from one moment.
//!   - setup holds its frame opaque until `onboarding.arrived`.
//!   - `done`: `onboarding.arrived` turns true (setup clears to its glass on
//!     that frame), setup takes focus, and the overlay closes.
//!
//! The same mechanism as Zeron's zgx app's `intro()` (MIT; see
//! THIRD_PARTY_NOTICES.md), written for Keplorayx.

use std::{cell::RefCell, rc::Rc};

use zgx_desktop::{WindowBounds, WindowContext, WindowHandle, WindowLevel, WindowOptions};
use zgx_script::{Shared, Value};

/// Setup's window: fixed, as a card is.
pub const SETUP_SIZE: (f64, f64) = (720., 520.);

/// The overlay the intro draws on.
pub fn overlay_window() -> WindowOptions {
    WindowOptions {
        title: "Keplorayx".into(),
        transparent: true,
        decorations: false,
        resizable: false,
        covers_display: true,
        level: WindowLevel::AlwaysOnTop,
        shadow: false,
        ..Default::default()
    }
}

/// Setup's window, as the onboarding's own mode opens it too.
pub fn setup_window() -> WindowOptions {
    WindowOptions {
        title: "Welcome to Keplorayx".into(),
        width: SETUP_SIZE.0,
        height: SETUP_SIZE.1,
        resizable: false,
        decorations: cfg!(target_os = "macos"),
        titlebar: zgx_desktop::Titlebar::Inset,
        glass: Some(zgx_desktop::Glass::new(zgx_desktop::GlassMaterial::UnderWindowBackground).plain()),
        ..Default::default()
    }
}

/// The intro in `cx` (the overlay); `open_setup` builds setup's window in
/// the window it is given (the same as the onboarding mode's).
pub fn install(
    cx: &mut WindowContext,
    host: &zgx_script::ScriptHost,
    open_setup: Rc<dyn Fn(&mut WindowContext) -> zgx_script::ScriptHost>,
) {
    cx.window.set_pointer_passthrough(true);
    host.shared("intro.width", SETUP_SIZE.0);
    host.shared("intro.height", SETUP_SIZE.1);
    let radius = host.shared("intro.radius", 0.);
    let setup: Rc<RefCell<Option<WindowHandle>>> = Rc::default();
    let arrived: Rc<RefCell<Option<Shared>>> = Rc::default();
    let epoch: Rc<RefCell<Option<Shared>>> = Rc::default();

    // The sky asks for the pointer once there is something to press.
    let window = cx.window.clone();
    host.action("intro.interactive", move |_| {
        window.set_pointer_passthrough(false);
        window.request_focus();
    });

    let (overlay, windows, opened) = (cx.window.clone(), cx.windows.clone(), setup.clone());
    let (bounds, viewport) = (cx.window_info.bounds.clone(), cx.viewport.clone());
    let (arriving, epoch_slot, opener) = (arrived.clone(), epoch.clone(), open_setup.clone());
    host.action("intro.prepare", move |rect| {
        if opened.borrow().is_some() {
            return;
        }
        let at = |key| rect.field(key).and_then(Value::as_f64).unwrap_or(0.);
        let (origin, physical_width) =
            bounds.with_untracked(|b| (b.position.unwrap_or((0, 0)), f64::from(b.size.0)));
        let scale = physical_width / viewport.with_untracked(|v| f64::from(v.0)).max(1.);
        let px = |points: f64| (points * scale).round();
        let options = WindowOptions {
            bounds: Some(WindowBounds {
                position: Some((origin.0 + px(at("x")) as i32, origin.1 + px(at("y")) as i32)),
                size: (px(at("w")) as u32, px(at("h")) as u32),
            }),
            visible: false,
            ..setup_window()
        };
        let (overlay, radius, arriving, epoch_slot, opener) =
            (overlay.clone(), radius.clone(), arriving.clone(), epoch_slot.clone(), opener.clone());
        let handle = windows.open(options, move |cx| {
            radius.set(cx.window_info.corner_radius.with_untracked(|r| *r).into());
            let host = opener(cx);
            *arriving.borrow_mut() = Some(host.shared("onboarding.arrived", false));
            *epoch_slot.borrow_mut() = Some(host.shared("onboarding.epoch", 0.));
            // Keys stay with the intro until it hands over.
            overlay.request_focus();
        });
        *opened.borrow_mut() = Some(handle);
    });

    let shown = setup.clone();
    host.action("intro.reveal", move |at| {
        if let (Some(epoch), Some(at)) = (&*epoch.borrow(), at.as_f64()) {
            epoch.set(Value::Num(at));
        }
        if let Some(window) = &*shown.borrow() {
            window.set_visible(true);
        }
    });

    let (overlay, windows) = (cx.window.clone(), cx.windows.clone());
    host.action("intro.done", move |_| {
        if let Some(arrived) = &*arrived.borrow() {
            arrived.set(Value::Bool(true));
        }
        match &*setup.borrow() {
            Some(window) => {
                window.set_visible(true);
                window.request_focus();
            }
            None => {
                // Skipped before the hand-off began: setup opens as it is.
                let opener = open_setup.clone();
                windows.open(setup_window(), move |cx| {
                    let host = opener(cx);
                    host.shared("onboarding.arrived", true);
                });
            }
        }
        overlay.close();
    });
}
