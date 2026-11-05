//! A window's zoom, as a browser's page zoom (Command or Control with plus,
//! minus and 0): wrapping text in a scroll view under a zoomable panel
//! (`View::zoomable`), the zoom, the panel's, the viewport in points and a
//! text's height in pixels printed as they change (`ZOOM`).
//! On macOS it sends its own window ⌘=, ⌘=, ⌘- and ⌘0, as the keyboard
//! would (`scripts/macos_zoom_probe.py`); `scripts/zoom_smoke.py` presses
//! the keys on X11. `--idle 1.5` zooms once and then does nothing, for
//! measuring what an idle zoomed window costs. `--inset`: an inset title
//! bar, with a mark beside the window's buttons in its 40-point strip (as
//! zterm's sidebar button): zoomed, the buttons move with the strip and
//! stay level with the mark (its reports add `buttons`, the close button's
//! centre below the window's top, in points). `--zoom 1.25` zooms to that
//! once the window shows, as the keys do, and goes on reporting.
use zgx::compose::prelude::*;
use zgx_desktop::{Application, Titlebar, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let inset = std::env::args().any(|arg| arg == "--inset");
    Application::new()
        .window(WindowOptions {
            title: "zgx zoom".into(),
            width: 480.,
            height: 360.,
            titlebar: if inset {
                Titlebar::Inset
            } else {
                Titlebar::Standard
            },
            ..Default::default()
        })
        .run(|cx| {
            let offset = cx.ui.signal(0_f32);
            let panel = cx.ui.signal(1_f32);
            let paragraphs: Vec<View> = (0..40)
                .map(|i| {
                    text(format!(
                        "Paragraph {i}: {}",
                        "words that wrap at the window's width ".repeat(4)
                    ))
                    .id(format!("p{i}"))
                    .text_wrap(true)
                })
                .collect();
            let mut page = column().w_full().h_full();
            if inset {
                page = page.child(
                    row()
                        .w_full()
                        .h(40.)
                        .shrink_0()
                        .items_center()
                        .pl(83.)
                        .window_drag()
                        .child(div().id("mark").w(28.).h(28.).rounded(7.).bg(rgb(0x4a90e2))),
                );
            }
            let view = cx.render(
                page.child(
                        row()
                            .id("panel")
                            .zoomable(panel.clone())
                            .p(8.)
                            .child(text("A zoomable panel").shrink_0()),
                    )
                    .child(
                        scroll(offset)
                            .w_full()
                            .grow()
                            .child(column().w_full().p(12.).gap(8.).children(paragraphs)),
                    ),
            );
            // `--idle 1.5`: zoomed once, then nothing runs (what an idle
            // window costs at a zoom).
            let idle = std::env::args()
                .skip_while(|arg| arg != "--idle")
                .nth(1)
                .and_then(|zoom| zoom.parse::<f64>().ok());
            if let Some(zoom) = idle {
                cx.ui.set_zoom(zoom);
                return;
            }
            if let Some(zoom) = std::env::args()
                .skip_while(|arg| arg != "--zoom")
                .nth(1)
                .and_then(|zoom| zoom.parse::<f64>().ok())
            {
                let ui = cx.ui.downgrade();
                cx.tasks.spawn(async move {
                    zgx::timer::sleep(std::time::Duration::from_millis(500)).await;
                    if let Some(ui) = ui.upgrade() {
                        ui.set_zoom(zoom);
                    }
                });
            }
            let ui = cx.ui.downgrade();
            let first = view.find("p0").unwrap();
            cx.tasks.spawn(async move {
                let mut last = String::new();
                loop {
                    let Some(ui) = ui.upgrade() else { return };
                    let line = {
                        let scene = ui.scene.borrow();
                        let root = scene.bounds(scene.root());
                        let size = match scene.kind(first) {
                            zgx::scene::NodeKind::Text { font_size, .. } => *font_size,
                            _ => 0.,
                        };
                        let line = format!(
                            "ZOOM zoom={:.2} panel={:.2} viewport={:.0}x{:.0} text_points={size:.1}",
                            ui.zoom(),
                            panel.get(),
                            root.width,
                            root.height
                        );
                        #[cfg(target_os = "macos")]
                        let line = match inset.then(keys::buttons_centre).flatten() {
                            Some(centre) => format!("{line} buttons={centre:.1}"),
                            None => line,
                        };
                        line
                    };
                    if line != last {
                        println!("{line}");
                        last = line;
                    }
                    drop(ui);
                    zgx::timer::sleep(std::time::Duration::from_millis(50)).await;
                }
            });
            #[cfg(target_os = "macos")]
            if std::env::args().any(|arg| arg == "--drive") {
                cx.tasks.spawn(async {
                    use std::time::Duration;
                    zgx::timer::sleep(Duration::from_millis(1500)).await;
                    for (key, code) in [("=", 24), ("=", 24), ("-", 27), ("0", 29)] {
                        keys::command(key, code);
                        zgx::timer::sleep(Duration::from_millis(600)).await;
                    }
                    println!("ZOOM_DONE");
                    std::process::exit(0);
                });
            }
        })
}

#[cfg(target_os = "macos")]
mod keys {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindowButton,
    };
    use objc2_foundation::{NSPoint, NSString};
    /// The probe window's close button's centre below the window's top, in
    /// points: where the inset title bar has the traffic lights.
    pub fn buttons_centre() -> Option<f64> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new()?);
        let window = app
            .windows()
            .iter()
            .find(|window| window.title().to_string() == "zgx zoom")?;
        let close = window.standardWindowButton(NSWindowButton::CloseButton)?;
        let frame = close.convertRect_toView(close.bounds(), None);
        Some(window.frame().size.height - (frame.origin.y + frame.size.height / 2.))
    }
    /// ⌘ with `characters` (key code `code`), pressed and released, sent to
    /// this app's key window as AppKit delivers a key: the user's keyboard
    /// and other apps hear nothing.
    pub fn command(characters: &str, code: u16) {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        // The key window, as AppKit delivers a key; or, the app not active (a
        // second run launched behind the first), the probe's window itself.
        let key = app.keyWindow();
        let Some(window) = key.clone().or_else(|| {
            app.windows()
                .iter()
                .find(|window| window.title().to_string() == "zgx zoom")
        }) else {
            return;
        };
        for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind,
                NSPoint::new(0., 0.),
                NSEventModifierFlags::Command,
                0.,
                window.windowNumber(),
                None,
                &NSString::from_str(characters),
                &NSString::from_str(characters),
                false,
                code,
            )
            .unwrap();
            match &key {
                Some(_) => app.sendEvent(&event),
                None => window.sendEvent(&event),
            }
        }
    }
}
