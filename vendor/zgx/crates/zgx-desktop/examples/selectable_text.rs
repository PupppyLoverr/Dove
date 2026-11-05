//! Selectable display text, as on a web page: one `.selectable(true)` and
//! the pointer selects across headings, paragraphs, list items and inline
//! spans; double and triple presses select words and paragraphs, Shift
//! extends, ⌘A / Ctrl+A selects all and ⌘C / Ctrl+C copies, as does the
//! context menu on selected text. Links and buttons inside still click,
//! and a drag from one selects. A drag held past the transcript's edge
//! scrolls it, and the selection outlives the rows it recycles.
use zgx::{compose::prelude::*, scene::Color, theme::tok};
use zgx_desktop::{Application, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smoke = std::env::args().any(|arg| arg == "--smoke-test");
    // macOS: copy all of it as Copy does, read this process's own
    // pasteboard back (its types and the HTML and plain text), print them
    // and quit. Reading what it wrote itself asks no one's permission.
    let pasteboard = std::env::args().any(|arg| arg == "--pasteboard-probe");
    // Find in page (Command-F) driven from outside: its status as it
    // changes, and the transcript's scroll (`find_in_page_smoke.py`).
    let find_probe = std::env::args().any(|arg| arg == "--find-probe");
    Application::new()
        .window(WindowOptions {
            title: "zgx selectable text".into(),
            width: 640.,
            height: 700.,
            ..Default::default()
        })
        .run(move |cx| {
            let clicks = cx.ui.signal(0_u32);
            let transcript = cx.ui.signal(0_f32);
            let transcript_offset = transcript.clone();
            let (count, link) = (clicks.clone(), clicks.clone());
            let item = |text: &str| {
                row()
                    .gap(8.)
                    .child(text_span_label("•"))
                    .child(zgx::compose::text(text).flex_grow(1.).text_wrap(true))
            };
            // What the window provides (its clipboard), for the probe.
            let platform: std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<zgx::platform::PlatformService>>>> =
                Default::default();
            let found = platform.clone();
            let view = cx.render(
                column()
                    .id("document")
                    .child(component(move |cx| {
                        *found.borrow_mut() = cx.try_service::<zgx::platform::PlatformService>();
                        div()
                    }))
                    .w_full()
                    .h_full()
                    .p(28.)
                    .gap(14.)
                    .text_size(15.)
                    .line_height(22.)
                    .selectable(true)
                    .child(
                        text("Selectable text")
                            .id("heading")
                            .text_size(26.)
                            .line_height(32.)
                            .font_bold(),
                    )
                    .child(
                        rich_text()
                            .id("intro")
                            .w_full()
                            .text_wrap(true)
                            .child(text_span("Drag across this paragraph and the ones below: "))
                            .child(text_span("one selection").font_bold())
                            .child(text_span(" runs through every text node in document order, "))
                            .child(text_span("inline spans").italic(true))
                            .child(text_span(" included. "))
                            .child(
                                text_span("This link")
                                    .text_color(Color(90, 160, 255, 255))
                                    .on_click(move || {
                                        link.update(|n| *n += 1);
                                    }),
                            )
                            .child(text_span(" still clicks.")),
                    )
                    .child(
                        column()
                            .gap(4.)
                            .child(item("Double-press a word, triple-press a paragraph."))
                            .child(item("Keep dragging after either to extend by words or paragraphs."))
                            .child(item("Shift-press extends from the anchor; ⌘A / Ctrl+A selects all.")),
                    )
                    .child(
                        row()
                            .gap(12.)
                            .items_center()
                            .child(
                                button()
                                    .id("button")
                                    .child("Press me")
                                    .on_click(move || {
                                        count.update(|n| *n += 1);
                                    }),
                            )
                            .child(text_signal(move || format!("Pressed {} times", clicks.get()))),
                    )
                    .child(
                        text("Text in a part marked unselectable is left out of copies.")
                            .text_color(tok::TEXT_TERTIARY)
                            .selectable(false),
                    )
                    .child(
                        text("Copy (⌘C / Ctrl+C) puts plain text on the clipboard: a line break between lines, a blank line between paragraphs.")
                            .id("last")
                            .w_full()
                            .text_wrap(true),
                    )
                    .child(
                        virtual_list(
                            transcript,
                            22.,
                            4,
                            || 10_000,
                            |index| index,
                            |_, line, _| {
                                row()
                                    .gap(12.)
                                    .child(text(format!("{:05}", line + 1)).w(52.).text_color(tok::TEXT_TERTIARY))
                                    .child(text(format!("A transcript line in a virtual list of 10,000 (line {})", line + 1)))
                            },
                        )
                        .id("transcript")
                        .w_full()
                        .h(176.)
                        .scrollbar(true),
                    ),
            );
            // Start with part of it selected (for screenshots).
            if std::env::args().any(|arg| arg == "--preselect") {
                let ui = cx.ui.downgrade();
                let (intro, last) = (view.find("intro").unwrap(), view.find("last").unwrap());
                cx.tasks.spawn(async move {
                    zgx::timer::sleep(std::time::Duration::from_millis(300)).await;
                    if let Some(ui) = ui.upgrade() {
                        let paragraph = ui.scene.borrow().children(intro)[0];
                        let anchor = zgx::selection::TextPoint {
                            node: paragraph,
                            offset: 5,
                        };
                        let focus = zgx::selection::TextPoint {
                            node: last,
                            offset: 40,
                        };
                        ui.select_text(anchor, focus);
                    }
                });
            }
            if pasteboard {
                let ui = cx.ui.downgrade();
                let window = cx.window.clone();
                let heading = view.find("heading").unwrap();
                let platform = platform.clone();
                cx.tasks.spawn(async move {
                    zgx::timer::sleep(std::time::Duration::from_millis(500)).await;
                    let Some(ui) = ui.upgrade() else { return };
                    assert!(ui.select_all_text(heading));
                    let (text, html) = (ui.selected_text(), ui.selected_html());
                    let platform = platform.borrow().clone().expect("a platform service");
                    let written = platform.0.set_clipboard_rich_text(text.clone(), html.clone());
                    println!("PASTEBOARD written={written}");
                    report_pasteboard(&text, &html);
                    window.close();
                });
            }
            if find_probe {
                let ui = cx.ui.downgrade();
                let offset = transcript_offset.clone();
                cx.tasks.spawn(async move {
                    let mut last = String::new();
                    loop {
                        let Some(ui) = ui.upgrade() else { return };
                        let s = ui.find_status();
                        // Text showing find's current mark, and any mark.
                        let current = ui.theme_scope().color_untracked(tok::FIND_CURRENT);
                        let (current_marks, marks) = {
                            let scene = ui.scene.borrow();
                            let marked: Vec<_> = scene.marked_text().collect();
                            let current = marked
                                .iter()
                                .filter(|id| scene.text_marks(**id).iter().any(|m| m.color == current))
                                .count();
                            (current, marked.len())
                        };
                        let line = format!(
                            "FIND open={} query={:?} count={} done={} current={:?} offset={:.0} marked={marks} current_marked={current_marks} selected={:?}",
                            s.open,
                            s.query,
                            s.count,
                            s.done,
                            s.current,
                            offset.get(),
                            ui.selected_text()
                        );
                        if line != last {
                            println!("{line}");
                            last = line;
                        }
                        drop(ui);
                        zgx::timer::sleep(std::time::Duration::from_millis(50)).await;
                    }
                });
                // macOS: the keys as the keyboard sends them, to this
                // window alone (`scripts/macos_find_probe.py`).
                #[cfg(target_os = "macos")]
                cx.tasks.spawn(async {
                    use std::time::Duration;
                    zgx::timer::sleep(Duration::from_millis(1500)).await;
                    keys::press("f", 3, true);
                    zgx::timer::sleep(Duration::from_millis(300)).await;
                    for c in "line 7777)".chars() {
                        keys::press(&c.to_string(), 0, false);
                        zgx::timer::sleep(Duration::from_millis(30)).await;
                    }
                    zgx::timer::sleep(Duration::from_millis(1500)).await;
                    keys::press("\r", 36, false);
                    zgx::timer::sleep(Duration::from_millis(300)).await;
                    keys::press("\u{1b}", 53, false);
                    zgx::timer::sleep(Duration::from_millis(500)).await;
                    println!("FIND_PROBE_DONE");
                    std::process::exit(0);
                });
            }
            if smoke {
                let ui = cx.ui.downgrade();
                let window = cx.window.clone();
                let find = |id: &str| view.find(id).unwrap();
                let (heading, intro, last) = (find("heading"), find("intro"), find("last"));
                let transcript = find("transcript");
                cx.tasks.spawn(async move {
                    for _ in 0..400 {
                        let Some(ui) = ui.upgrade() else { return };
                        {
                            let scene = ui.scene.borrow();
                            let bounds = |id| {
                                let b: zgx::scene::Rect = scene.bounds(id);
                                format!("{:.1},{:.1},{:.1},{:.1}", b.x, b.y, b.width, b.height)
                            };
                            println!(
                                "SELECTABLE heading={} intro={} last={} transcript={} text={:?}",
                                bounds(heading),
                                bounds(intro),
                                bounds(last),
                                bounds(transcript),
                                ui.selected_text(),
                            );
                        }
                        zgx::timer::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    window.close();
                });
            }
        })
}

fn text_span_label(text: &str) -> View {
    zgx::compose::text(text).text_color(tok::TEXT_TERTIARY)
}

/// This process's pasteboard: its types, and whether the HTML and the plain
/// text are what was copied.
#[cfg(target_os = "macos")]
fn report_pasteboard(text: &str, html: &str) {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;
    let board = NSPasteboard::generalPasteboard();
    let types: Vec<String> = board
        .types()
        .map(|types| types.iter().map(|t| t.to_string()).collect())
        .unwrap_or_default();
    let read = |kind: &str| {
        board
            .stringForType(&NSString::from_str(kind))
            .map(|s| s.to_string())
    };
    let (got_html, got_text) = (read("public.html"), read("public.utf8-plain-text"));
    println!("PASTEBOARD types={types:?}");
    println!(
        "PASTEBOARD html_matches={} html_bytes={} html_start={:?}",
        got_html.as_deref().is_some_and(|got| got.contains(html)),
        got_html.as_ref().map_or(0, String::len),
        got_html
            .as_deref()
            .map(|h| h.chars().take(120).collect::<String>()),
    );
    println!(
        "PASTEBOARD text_matches={} text_lines={}",
        got_text.as_deref() == Some(text),
        got_text.as_deref().map_or(0, |t| t.lines().count()),
    );
}
#[cfg(not(target_os = "macos"))]
fn report_pasteboard(_: &str, _: &str) {
    println!("PASTEBOARD unsupported: macOS only");
}

#[cfg(target_os = "macos")]
mod keys {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::{NSPoint, NSString};
    /// `characters` (key code `code`) pressed and released, with Command
    /// if `command`, sent to this app's key window as AppKit delivers a
    /// key: the user's keyboard and other apps hear nothing.
    pub fn press(characters: &str, code: u16, command: bool) {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        let Some(window) = app.keyWindow() else {
            return;
        };
        let flags = if command {
            NSEventModifierFlags::Command
        } else {
            NSEventModifierFlags::empty()
        };
        for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind,
                NSPoint::new(0., 0.),
                flags,
                0.,
                window.windowNumber(),
                None,
                &NSString::from_str(characters),
                &NSString::from_str(characters),
                false,
                code,
            )
            .unwrap();
            app.sendEvent(&event);
        }
    }
}
