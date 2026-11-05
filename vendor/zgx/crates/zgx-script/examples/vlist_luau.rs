//! The variable-height list comparison scene (see zgx_workload::vlist) in
//! Luau: 100,000 chat rows of wrapped text in `ui.list { measured = true }`
//! (examples/vlist_luau/app.luau), each row's text read from the host with
//! `ui.call_sync`. The host moves the list by the scene's scroll or fling
//! each tick, as the Rust, GPUI Fast and MyGo adapters' hosts do; a jump is
//! the script's `scroll_to_row`.
//! Run with ZGX_MODE=fling ZGX_SECONDS=10.
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};
use zgx_desktop::{Application, FontData, WindowOptions};
use zgx_script::{ScriptHost, Value};
use zgx_workload::{
    PERIOD,
    vlist::{self, Mode},
};

/// The estimate of a row not yet measured: three lines and no image, as
/// the Rust adapter's.
const ESTIMATE: f32 = 2. * vlist::PAD.1 + vlist::LINE_HEIGHT + vlist::GAP + 3. * vlist::LINE_HEIGHT;

/// A Luau table of string keys (a `Value::Map` is sorted by key).
fn map(mut entries: Vec<(&str, Value)>) -> Value {
    entries.sort_by_key(|(k, _)| *k);
    Value::Map(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
}

fn color(rgb: u32) -> Value {
    Value::Str(format!("#{rgb:06x}").into())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = Mode::from_env();
    let seconds: f64 = std::env::var("ZGX_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30.);
    // The script goes beside the app's temporary files: the runner launches
    // a copy staged in /tmp, which reads nothing from the checkout.
    let script = std::env::temp_dir().join(format!("zgx-vlist-luau-{}", std::process::id()));
    std::fs::create_dir_all(&script)?;
    let script = script.join("app.luau");
    std::fs::write(&script, include_str!("vlist_luau/app.luau"))?;
    let mut application = Application::new();
    if let Some(bytes) = zgx_workload::font_file() {
        application = application.font(FontData::new(std::sync::Arc::<[u8]>::from(bytes))?);
    }
    application
        .window(WindowOptions {
            title: "zgx vlist · Luau".into(),
            width: vlist::WIDTH as f64,
            height: vlist::HEIGHT as f64,
            resizable: false,
            ..Default::default()
        })
        .run(move |cx| {
            let host = ScriptHost::new(&cx.ui.runtime, script.clone());
            host.shared(
                "scene",
                map(vec![
                    ("rows", Value::Num(vlist::ROWS as f64)),
                    ("estimate", Value::Num(ESTIMATE.into())),
                    ("font", vlist::FONT.into()),
                    ("font_size", Value::Num(vlist::FONT_SIZE.into())),
                    ("line_height", Value::Num(vlist::LINE_HEIGHT.into())),
                    ("pad_x", Value::Num(vlist::PAD.0.into())),
                    ("pad_y", Value::Num(vlist::PAD.1.into())),
                    ("gap", Value::Num(vlist::GAP.into())),
                    ("text_width", Value::Num(vlist::TEXT_WIDTH.into())),
                    ("image_w", Value::Num(vlist::IMAGE.0.into())),
                    ("image_h", Value::Num(vlist::IMAGE.1.into())),
                    ("background", color(vlist::BACKGROUND)),
                    (
                        "row_colors",
                        Value::List(
                            [color(vlist::ROW_COLORS[0]), color(vlist::ROW_COLORS[1])].into(),
                        ),
                    ),
                    ("author", color(vlist::AUTHOR)),
                    ("text", color(vlist::TEXT)),
                    ("muted", color(vlist::MUTED)),
                    ("image", color(vlist::IMAGE_COLOR)),
                    ("probe", Value::Bool(mode == Mode::Probe)),
                ]),
            );
            let offset = host.shared("offset", 0.);
            // A jump: the script scrolls its list to the row (`scroll_to_row`).
            let jump = host.shared("jump", Value::List([Value::Num(0.), Value::Num(0.)].into()));
            // Row `index` (from 0), read as it is built.
            host.function("row", |index| {
                let index = index.as_f64().unwrap_or(0.) as usize;
                map(vec![
                    ("author", vlist::author(index).as_str().into()),
                    ("time", vlist::time(index).as_str().into()),
                    ("text", vlist::text(index).as_str().into()),
                    ("image", Value::Bool(vlist::has_image(index))),
                ])
            });
            let _ = host.reload();
            cx.render(host.view());

            let ticks = Rc::new(Cell::new(0_u64));
            cx.on_closed({
                let ticks = ticks.clone();
                move || eprintln!("workload_ticks={}", ticks.get())
            });
            let window = cx.window.clone();
            let jumps = Rc::new(Cell::new(0_u32));
            let jump_to = move |row: usize| {
                jumps.set(jumps.get() + 1);
                jump.set(Value::List(
                    [Value::Num(jumps.get().into()), Value::Num(row as f64)].into(),
                ));
            };
            if mode == Mode::Probe {
                // Probe mode: each row reports its laid-out height (on_resize).
                let heights: Rc<RefCell<HashMap<usize, f32>>> = Rc::default();
                host.function("height", {
                    let heights = heights.clone();
                    move |arg| {
                        if let [row, height] = arg.list() {
                            let (row, height) =
                                (row.as_f64().unwrap_or(0.), height.as_f64().unwrap_or(0.));
                            heights.borrow_mut().insert(row as usize, height as f32);
                        }
                        Value::Nil
                    }
                });
                // Each probe row at the top in turn, measured after a few frames.
                cx.tasks.spawn(async move {
                    let mut measured = Vec::new();
                    for row in vlist::probe_rows() {
                        jump_to(row);
                        zgx::timer::sleep(PERIOD * 12).await;
                        measured.push((row, heights.borrow().get(&row).copied().unwrap_or(0.)));
                    }
                    eprintln!("{}", vlist::probe_report(&measured, vlist::WIDTH));
                    window.close();
                });
                return;
            }
            cx.tasks.spawn(async move {
                let start = Instant::now();
                let end = (seconds > 0.).then(|| start + Duration::from_secs_f64(seconds));
                let mut next = start + PERIOD;
                // Where the list ends, by the estimate: 100k rows take minutes to reach.
                let max = (vlist::ROWS as f32 * ESTIMATE - vlist::HEIGHT).max(0.);
                loop {
                    zgx::timer::sleep_until(end.map_or(next, |end| end.min(next))).await;
                    if end.is_some_and(|end| Instant::now() >= end) {
                        window.close();
                        break;
                    }
                    let tick = ticks.get() + 1;
                    ticks.set(tick);
                    match vlist::jump(tick) {
                        Some(row) if mode == Mode::Jump => jump_to(row),
                        _ => {
                            let now = offset.with_untracked(|v| v.as_f64().unwrap_or(0.)) as f32;
                            let moved = now + vlist::delta(mode, tick);
                            // At the end, back to the top.
                            offset.set(Value::Num(if moved > max { 0. } else { moved.into() }));
                        }
                    }
                    next += PERIOD;
                    let now = Instant::now();
                    if next <= now {
                        next = now + PERIOD;
                    }
                }
            });
        })
}
