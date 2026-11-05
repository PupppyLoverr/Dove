//! "Your agents, anywhere": a first-launch intro in the manner of Dia's,
//! played over whatever is open. No window shows: a transparent, frameless
//! window over the whole display, above other windows, lets the pointer
//! through until there is something to press.
//!
//! The scene is `examples/agents_intro/app.luau`: painted plates of a sea at
//! night, blue hour and sunrise, crossfading as a time-lapse, materialising
//! over the desktop out of characters tinted by the picture beneath them
//! (`characters`, on the GPU, its front animated through `shader_params`),
//! clouds drifting in at their own speeds, and the agents' marks flying in
//! on tilted glass cards, each drifting on its own period. Every value moves
//! on the compositor: no layout and no script per frame. Escape skips it;
//! edit and save to reload.
//!
//! Run: cargo run --release -p zgx-desktop --example agents_intro
use std::{path::Path, time::Duration};
use zgx_desktop::{Application, FontData, WindowLevel, WindowOptions};
use zgx_script::ScriptHost;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/agents_intro/app.luau");
    let mut application = Application::new();
    // Instrument Serif for the title (zeron.sh's), Geist for the rest.
    for bytes in [
        &include_bytes!("support/fonts/InstrumentSerif-Regular.ttf")[..],
        include_bytes!("support/fonts/InstrumentSerif-Italic.ttf"),
        include_bytes!("support/fonts/Geist.ttf"),
        include_bytes!("support/fonts/Geist-Medium.ttf"),
    ] {
        application = application.font(FontData::new(std::sync::Arc::<[u8]>::from(bytes))?);
    }
    application
        .window(WindowOptions {
            title: "Zeron".into(),
            transparent: true,
            decorations: false,
            resizable: false,
            covers_display: true,
            level: WindowLevel::AlwaysOnTop,
            shadow: false,
            ..Default::default()
        })
        .run(move |cx| {
            // The desktop stays usable under the scene until the card asks for
            // the pointer.
            cx.window.set_pointer_passthrough(true);
            let host = ScriptHost::new(&cx.ui.runtime, script).app_id("agents-intro");
            let window = cx.window.clone();
            host.action("interactive", move |_| {
                window.set_pointer_passthrough(false);
                window.request_focus();
            });
            let window = cx.window.clone();
            host.action("close", move |_| window.close());
            let _ = host.reload();
            cx.tasks.spawn(host.watch(Duration::from_millis(250)));
            cx.render(host.view());
        })?;
    Ok(())
}
