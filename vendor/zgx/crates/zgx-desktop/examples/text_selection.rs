//! Two editors to select, copy and paste in: a line of words and two lines
//! of text, laid out as `primary_selection`'s. `windows_text_smoke.py`
//! double- and triple-clicks them at the system's timings and copies and
//! pastes through the clipboard. Prints the editors' text as it closes.
use zgx::compose::prelude::*;
use zgx_desktop::{Application, WindowOptions};

fn field(view: View) -> View {
    view.w(440.0).p(10.0).rounded(6.0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "zgx text selection".into(),
            width: 480.0,
            height: 260.0,
            ..Default::default()
        })
        .run(move |cx| {
            let words = cx.ui.signal("select these words".to_owned());
            let lines = cx.ui.signal("first line\nsecond line".to_owned());
            cx.render(
                column()
                    .p(20.0)
                    .gap(16.0)
                    .text_size(16.0)
                    .child(field(
                        text_input("Words", words.clone()).id("words").h(44.0),
                    ))
                    .child(field(text_area("Lines", lines.clone()).id("lines").h(96.0))),
            );
            cx.on_closed(move || {
                println!("TEXT words={:?} lines={:?}", words.get(), lines.get());
            });
        })
}
