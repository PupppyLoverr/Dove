//! The microbenchmarks' scenes in Luau (`micro_bench` with `ENGINE=luau`):
//! the same views as the Rust ones, built by a script through `ScriptHost`,
//! each row's text read from the host (`ui.call_sync`) as a Luau app reads
//! host data. A state operation runs the script's handler (`counter:add`);
//! a scroll step is the host moving the list, as a window's input does.
use zgx_script::{ScriptHost, Shared, Value};
use zgx_workload::{
    micro::{self, *},
    vlist,
};

fn color(rgb: u32) -> String {
    format!("\"#{rgb:06x}\"")
}

/// The state scene: a static grid under one text bound to a counter.
fn state_source() -> String {
    format!(
        r#"local counter = ui.state("counter", 0)
ui.command {{ id = "tick", run = function() counter:add(1) end }}
local lines = {{}}
for r = 0, {rows} - 1 do
    local cells = {{}}
    for c = 0, {columns} - 1 do
        cells[c + 1] = ui.text {{ tostring(r * {columns} + c), w = {cell_w}, h = {cell_h} }}
    end
    lines[r + 1] = ui.row {{ gap = 2, cells }}
end
return ui.column {{
    w_full = true, h_full = true, p = 8, gap = 6,
    bg = {background}, font = "{font}", text_size = {size}, color = {text},
    ui.text {{ "Count: {{counter}}", h = 20 }},
    ui.column {{ gap = 2, lines }},
}}"#,
        rows = STATIC_ROWS,
        columns = STATIC_COLUMNS,
        cell_w = CELL.0,
        cell_h = CELL.1,
        background = color(vlist::BACKGROUND),
        font = zgx_workload::FONT,
        size = micro::FONT_SIZE,
        text = color(vlist::TEXT),
    )
}

/// The fixed-height list: `ROWS` rows of `fixed_label`.
fn fixed_source() -> String {
    format!(
        r#"local offset = ui.shared("offset", 0)
local colors = {{ {even}, {odd} }}
return ui.column {{
    w_full = true, h_full = true,
    bg = {background}, font = "{font}", text_size = {size}, color = {text},
    ui.list {{
        count = {rows}, height = {row_height}, overscan = 2, offset = offset,
        w_full = true, h_full = true,
        row = function(i)
            return ui.row {{
                w_full = true, h = {row_height}, px = 12, items = "center",
                bg = colors[(i - 1) % 2 + 1],
                ui.text {{ ui.call_sync("fixed", i - 1), literal = true }},
            }}
        end,
    }},
}}"#,
        even = color(vlist::ROW_COLORS[0]),
        odd = color(vlist::ROW_COLORS[1]),
        background = color(vlist::BACKGROUND),
        font = zgx_workload::FONT,
        size = vlist::FONT_SIZE,
        text = color(vlist::TEXT),
        rows = ROWS,
        row_height = FIXED_ROW,
    )
}

/// The variable-height list: the vlist chat rows, measured as they mount.
fn variable_source(estimate: f32) -> String {
    format!(
        r#"local offset = ui.shared("offset", 0)
local colors = {{ {even}, {odd} }}
local function message(i)
    local row = ui.call_sync("row", i - 1)
    return ui.column {{
        w_full = true, px = {pad_x}, py = {pad_y}, gap = {gap},
        bg = colors[(i - 1) % 2 + 1],
        ui.row {{
            gap = 8, h = {line},
            ui.text {{ row.author, literal = true, color = {author} }},
            ui.text {{ row.time, literal = true, color = {muted} }},
        }},
        ui.text {{ row.text, literal = true, wrap = true, w = {text_width} }},
        if row.image then ui.div {{ w = {image_w}, h = {image_h}, bg = {image} }} else nil,
    }}
end
return ui.column {{
    w_full = true, h_full = true,
    bg = {background}, font = "{font}", text_size = {size}, line_height = {line},
    color = {text},
    ui.list {{
        count = {rows}, measured = true, height = {estimate}, overscan = 2,
        offset = offset, w_full = true, h_full = true, row = message,
    }},
}}"#,
        even = color(vlist::ROW_COLORS[0]),
        odd = color(vlist::ROW_COLORS[1]),
        pad_x = vlist::PAD.0,
        pad_y = vlist::PAD.1,
        gap = vlist::GAP,
        line = vlist::LINE_HEIGHT,
        author = color(vlist::AUTHOR),
        muted = color(vlist::MUTED),
        text_width = vlist::TEXT_WIDTH,
        image_w = vlist::IMAGE.0,
        image_h = vlist::IMAGE.1,
        image = color(vlist::IMAGE_COLOR),
        background = color(vlist::BACKGROUND),
        font = zgx_workload::FONT,
        size = vlist::FONT_SIZE,
        text = color(vlist::TEXT),
        rows = ROWS,
    )
}

/// Scene `scene`'s host, loaded, and what one operation does to it.
pub fn host(
    runtime: &zgx::reactive::Runtime,
    scene: &str,
    estimate: f32,
) -> (ScriptHost, Box<dyn FnMut(usize)>) {
    let source = match scene {
        "state" => state_source(),
        "vscroll-fixed" => fixed_source(),
        "vscroll-variable" => variable_source(estimate),
        other => panic!("SCENE {other}: state, vscroll-fixed or vscroll-variable"),
    };
    let host = ScriptHost::inline(runtime, "micro", source);
    host.function("fixed", |index| {
        fixed_label(index.as_f64().unwrap_or(0.) as usize)
            .as_str()
            .into()
    });
    host.function("row", |index| {
        let index = index.as_f64().unwrap_or(0.) as usize;
        Value::record([
            ("author", vlist::author(index).as_str().into()),
            ("time", vlist::time(index).as_str().into()),
            ("text", vlist::text(index).as_str().into()),
            ("image", Value::Bool(vlist::has_image(index))),
        ])
    });
    let offset: Shared = host.shared("offset", 0.);
    host.reload().expect("the scene's script loads");
    let step: Box<dyn FnMut(usize)> = match scene {
        "state" => {
            let host = host.clone();
            Box::new(move |_| {
                host.run_command("tick");
            })
        }
        _ => {
            let end = if scene == "vscroll-fixed" {
                ROWS as f32 * FIXED_ROW - VIEWPORT
            } else {
                ROWS as f32 * estimate - VIEWPORT
            };
            Box::new(move |_| {
                // Past the end, back to the top.
                let next = offset.with_untracked(|v| v.as_f64().unwrap_or(0.)) as f32 + STEP;
                offset.set(Value::Num(if next > end { 0. } else { next.into() }));
            })
        }
    };
    (host, step)
}
