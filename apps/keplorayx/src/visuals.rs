//! Agent visuals (`show_visual`), ported from Zeron's src/visuals.rs (MIT,
//! THIRD_PARTY_NOTICES.md): crates/visual (Zeron's sandboxed runtime) runs
//! the agent's Luau script, its tree handed to `ds/visual` as plain data,
//! and the reader's input routed back to the script.
//!
//! A visual reaches a thread as the `show_visual` tool's call: the bridge
//! (src/mcp.rs) keeps its script in `$DOVE_HOME/visuals/<id>-visual.luau`,
//! and the transcript names it by id and path. The script runs exactly as
//! Zeron runs it: no I/O, its memory capped, each call on a time budget,
//! `math.random` seeded from the visual's id.
//!
//! The calls:
//!   visuals.open { id, path, title, source? }   (`ui.call`) reads and starts it, once
//!   visuals.get id                              (`ui.call_sync`) { state, title, error?, tree? }
//!   visuals.event { id, h, arg? }               (`ui.action`) a control's handler, then the view again
//!   visuals.draw { id, h, w, ht, t, px?, py?, down? }  a canvas's drawing
//!   visuals.frame                               3D scenes and shaders: not drawn here yet (Zeron
//!                                               renders them on the GPU, crates/visual-gpu)
//! And the shared state `visuals`, `{ [id] = { rev, gen } }`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use zeron_visual as zv;
use zgx_script::{ScriptHost, Shared, Value};

/// Time spent drawing canvases, in nanoseconds (the cost probe's).
static DRAW_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The most a visual's files may be in all (crates/visual's own cap, and a
/// little for the manifest).
const MAX_READ: usize = zv::assets::MAX_TOTAL_ASSET_BYTES + zv::MAX_SCRIPT_BYTES + 64 * 1024;
/// A scene's or shader's frame at most this many device pixels a point
/// (the shipping app's mobile cap): sharp at 2x, never a 3x readback.

#[derive(Clone, Debug, PartialEq)]
enum Load {
    Loading,
    Ready,
    Failed(String),
}

/// A visual's live program and what it last showed.
#[allow(dead_code)]
struct Entry {
    title: String,
    load: Load,
    runtime: Option<zv::Visual>,
    tree: Option<zv::Node>,
    /// The tree as the scripts get it, made once per shape.
    value: Value,
    /// The latest handler or draw failure; the last good tree stays.
    error: Option<String>,
    rev: u64,
    generation: u64,
    /// Scenes' drawings, by node path: the render they were drawn at.
    scenes: HashMap<String, (u64, Arc<zv::Scene>)>,
    /// Shaders' frame counts and last times, by node path.
    shaders: HashMap<String, (u32, f64)>,
}

impl Entry {
    fn new(title: String) -> Self {
        Self {
            title,
            load: Load::Loading,
            runtime: None,
            tree: None,
            value: Value::Nil,
            error: None,
            rev: 0,
            generation: 0,
            scenes: HashMap::new(),
            shaders: HashMap::new(),
        }
    }

    /// Run `source`'s setup and first render, seeded from `id`.
    fn start(&mut self, id: &str, source: &str, assets: zv::Assets) {
        match zv::Visual::load(source, zv::seed_for(id)) {
            Ok(runtime) => {
                self.runtime = Some(runtime.with_assets(assets));
                self.load = Load::Ready;
                self.render();
            }
            Err(error) => {
                eprintln!("keplorayx: visual {id} failed to load: {error}");
                self.load = Load::Failed(error.message);
                self.rev += 1;
            }
        }
    }

    /// Run the view again. Its shape counts as changed only if the tree is
    /// another; every render is a new generation (a canvas reading state
    /// the handler changed draws again with the same tree).
    fn render(&mut self) {
        let Some(runtime) = self.runtime.as_mut() else { return };
        self.generation += 1;
        match runtime.render() {
            Ok(tree) => {
                let had_error = self.error.take().is_some();
                if self.tree.as_ref() != Some(&tree) {
                    self.value = node_value(&tree, "0");
                    self.tree = Some(tree);
                    self.rev += 1;
                } else if had_error {
                    self.rev += 1;
                }
            }
            Err(error) => self.fail(error),
        }
    }

    fn fail(&mut self, error: zv::VisualError) {
        if self.error.as_deref() != Some(error.message.as_str()) {
            self.error = Some(error.message);
            self.rev += 1;
        }
    }

    /// A control's handler, then the view again.
    fn event(&mut self, handler: u32, arg: zv::HandlerArg) {
        let Some(runtime) = self.runtime.as_mut() else { return };
        match runtime.call(zv::HandlerId(handler), arg) {
            Ok(()) => self.render(),
            Err(error) => self.fail(error),
        }
    }

    fn snapshot(&self) -> Value {
        let state = match &self.load {
            Load::Loading => "loading",
            Load::Ready => "ready",
            Load::Failed(_) => "failed",
        };
        let error = match &self.load {
            Load::Failed(why) => Some(why.as_str()),
            _ => self.error.as_deref(),
        };
        let mut fields = vec![("state", Value::from(state)), ("title", Value::from(self.title.as_str()))];
        if let Some(error) = error {
            fields.push(("error", Value::from(error)));
        }
        if self.tree.is_some() {
            fields.push(("tree", self.value.clone()));
        }
        Value::record(fields)
    }

    /// A canvas's drawing.
    fn draw(&mut self, arg: &Value) -> Value {
        let Some(runtime) = self.runtime.as_mut() else {
            return Value::record([("error", Value::from("not running"))]);
        };
        let n = |k: &str| arg.field(k).and_then(Value::as_f64);
        let (w, h) = (n("w").unwrap_or(0.) as f32, n("ht").unwrap_or(0.) as f32);
        let pointer = match (n("px"), n("py")) {
            (Some(x), Some(y)) => Some(zv::Pointer {
                x: x as f32,
                y: y as f32,
                down: arg.field("down").is_some_and(Value::truthy),
            }),
            _ => None,
        };
        let handler = zv::HandlerId(n("h").unwrap_or(-1.) as u32);
        match runtime.draw_canvas_traced(handler, w, h, n("t").unwrap_or(0.), pointer) {
            Ok(drawing) => Value::record([
                ("cmds", Value::List(drawing.commands.iter().map(command_value).collect::<Vec<_>>().into())),
                ("pointer", Value::Bool(drawing.reads_pointer)),
            ]),
            Err(error) => Value::record([("error", Value::from(error.message.as_str()))]),
        }
    }

    /// The node at `path` ("0.2.1"): its children by index.
    #[allow(dead_code)]
    fn node_at(&self, path: &str) -> Option<&zv::Node> {
        let mut node = self.tree.as_ref()?;
        for step in path.split('.').skip(1) {
            node = zv::children(node).nth(step.parse().ok()?)?;
        }
        Some(node)
    }
}

/// Every visual this app shows, and each window's shared `visuals`.
#[derive(Default)]
struct Visuals {
    entries: RefCell<HashMap<String, Entry>>,
    shared: RefCell<Vec<Shared>>,
}

thread_local! {
    static VISUALS: Rc<Visuals> = Rc::default();
}

impl Visuals {
    fn revs(&self) -> Value {
        let entries = self.entries.borrow();
        Value::record(entries.iter().map(|(id, e)| {
            (
                id.as_str(),
                Value::record([("rev", Value::Num(e.rev as f64)), ("gen", Value::Num(e.generation as f64))]),
            )
        }))
    }

    fn publish(&self) {
        let value = self.revs();
        for shared in self.shared.borrow().iter() {
            shared.set(value.clone());
        }
    }

    fn with<R>(&self, id: &str, f: impl FnOnce(&mut Entry) -> R) -> Option<R> {
        let mut entries = self.entries.borrow_mut();
        entries.get_mut(id).map(f)
    }
}

/// The scripts' calls, on every window but the intro's.
pub fn install(cx: &mut zgx_desktop::WindowContext, host: &ScriptHost) {
    // `ui.window.shown`: a visual stops animating while its window is
    // hidden, minimized or covered.
    host.window_state(&cx.ui);
    register(host);
}

/// The calls on `host` (a test's, with no window, too).
pub fn register(host: &ScriptHost) {
    let visuals = VISUALS.with(Rc::clone);
    visuals.shared.borrow_mut().push(host.shared("visuals", visuals.revs()));

    let v = visuals.clone();
    host.async_action("visuals.open", move |arg| {
        let v = v.clone();
        async move { open(v, arg).await }
    });
    let v = visuals.clone();
    host.function("visuals.get", move |arg| {
        let id = as_text(&arg);
        v.with(&id, |e| e.snapshot()).unwrap_or(Value::Nil)
    });
    let v = visuals.clone();
    host.action("visuals.event", move |arg| {
        let id = text(arg.field("id"));
        let handler = arg.field("h").and_then(Value::as_f64).unwrap_or(-1.);
        if handler < 0. {
            return;
        }
        let changed = v
            .with(&id, |e| {
                let before = (e.rev, e.generation);
                e.event(handler as u32, handler_arg(arg.field("arg")));
                (e.rev, e.generation) != before
            })
            .unwrap_or(false);
        if changed {
            v.publish();
        }
    });
    let v = visuals.clone();
    host.function("visuals.draw", move |arg| {
        let id = text(arg.field("id"));
        let start = std::time::Instant::now();
        let drawn = v.with(&id, |e| e.draw(&arg)).unwrap_or(Value::Nil);
        DRAW_NS.fetch_add(start.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        drawn
    });
    // 3D scenes and shaders: Zeron draws them on the GPU (crates/visual-gpu),
    // not ported yet; they say so where they'd be.
    host.async_action("visuals.frame", |_| async move {
        Err::<Value, String>("3D scenes and shaders aren't drawn in Keplorayx yet".into())
    });
    host.action("visuals.release", |_| {});
}

/// (For tests: how many frames each shader of visual `id` has drawn, and
/// which scenes have a drawing, by node path.)
#[cfg(test)]
pub fn surfaces(id: &str) -> (Vec<(String, u32)>, Vec<String>) {
    VISUALS.with(|v| {
        v.with(id, |e| {
            (
                e.shaders.iter().map(|(p, (n, _))| (p.clone(), *n)).collect(),
                e.scenes.keys().cloned().collect(),
            )
        })
        .unwrap_or_default()
    })
}

/// (For tests: visual `id` started from `source`, as a part naming it
/// would find it already read.)
#[cfg(test)]
pub fn preload(id: &str, title: &str, source: &str) {
    VISUALS.with(|v| {
        let mut e = Entry::new(title.into());
        e.start(id, source, zv::Assets::default());
        v.entries.borrow_mut().insert(id.into(), e);
        v.publish();
    });
}

fn text(value: Option<&Value>) -> String {
    value.map(as_text).unwrap_or_default()
}

/// `visuals.open`: read the script and its files, then start it.
async fn open(v: Rc<Visuals>, arg: Value) -> Result<Value, String> {
    let id = text(arg.field("id"));
    if id.is_empty() {
        return Err("no visual".into());
    }
    let title = text(arg.field("title"));
    {
        let mut entries = v.entries.borrow_mut();
        match entries.get(&id).map(|e| &e.load) {
            // Read, or being read: once.
            Some(Load::Loading | Load::Ready) => return Ok(Value::Nil),
            // (One that failed is read again, as the chat opens again.)
            Some(Load::Failed(_)) | None => {
                entries.insert(id.clone(), Entry::new(title));
            }
        }
    }
    v.publish();
    let loaded = match arg.field("source") {
        Some(source) => Ok((as_text(source), zv::Assets::default())),
        None => read_visual(&text(arg.field("path")), arg.field("device").map(as_text)).await,
    };
    v.with(&id, |e| match loaded {
        Ok((source, assets)) => e.start(&id, &source, assets),
        Err(why) => {
            eprintln!("keplorayx: visual {id} unavailable: {why}");
            e.load = Load::Failed(why);
            e.rev += 1;
        }
    });
    v.publish();
    Ok(Value::Nil)
}

/// A visual's script and its files (listed in `{stem}-assets.json` beside
/// it, absent for one without), as the shipping app reads them.
async fn read_visual(path: &str, device: Option<String>) -> Result<(String, zv::Assets), String> {
    if !path.ends_with("-visual.luau") {
        return Err("That isn't a visual's script.".into());
    }
    let device = device.filter(|d| !d.is_empty());
    let bytes = read_file(path, device.as_deref()).await?;
    let source = String::from_utf8(bytes).map_err(|_| "The visual's script isn't text.".to_string())?;
    let mut files = HashMap::new();
    let stem = path.trim_end_matches("-visual.luau");
    if let Ok(manifest) = read_file(&format!("{stem}-assets.json"), device.as_deref()).await
        && let Ok(names) = serde_json::from_slice::<Vec<String>>(&manifest)
    {
        for name in names.into_iter().filter(|n| zv::assets::valid_name(n)).take(zv::assets::MAX_ASSETS) {
            if let Ok(bytes) = read_file(&format!("{stem}-asset-{name}"), device.as_deref()).await {
                files.insert(name, Arc::<[u8]>::from(bytes));
            }
        }
    }
    Ok((source, zv::Assets::new(files)))
}

/// A visual's file, from the disk: only inside the visuals folder.
async fn read_file(path: &str, _device: Option<&str>) -> Result<Vec<u8>, String> {
    let dir = dove_store::default_root().join("visuals");
    let path = std::path::Path::new(path);
    let inside = path.canonicalize().ok().zip(dir.canonicalize().ok()).is_some_and(|(p, d)| p.starts_with(d));
    if !inside {
        return Err("That visual isn't one of Keplorayx's.".into());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("The visual can't be read: {e}"))?;
    if bytes.len() > MAX_READ {
        return Err("The visual is too large.".into());
    }
    Ok(bytes)
}

fn as_text(value: &Value) -> String {
    match value {
        Value::Str(s) => s.to_string(),
        Value::Num(n) => n.to_string(),
        _ => String::new(),
    }
}

/// A handler's argument from what the script sent.
fn handler_arg(value: Option<&Value>) -> zv::HandlerArg {
    match value {
        None | Some(Value::Nil) => zv::HandlerArg::None,
        Some(Value::Bool(b)) => zv::HandlerArg::Bool(*b),
        Some(Value::Num(n)) => zv::HandlerArg::Number(*n),
        Some(point @ Value::Map(_)) => {
            let n = |k: &str| point.field(k).and_then(Value::as_f64).unwrap_or(0.) as f32;
            zv::HandlerArg::Point(n("x"), n("y"))
        }
        Some(other) => zv::HandlerArg::Text(as_text(other)),
    }
}

// ---- the tree as data -------------------------------------------------------

fn num(n: impl Into<f64>) -> Value {
    Value::Num(n.into())
}

fn list(items: Vec<Value>) -> Value {
    Value::List(items.into())
}

fn token_name(token: zv::Token) -> String {
    match token {
        zv::Token::Text => "text".into(),
        zv::Token::Muted => "muted".into(),
        zv::Token::Faint => "faint".into(),
        zv::Token::Accent => "accent".into(),
        zv::Token::OnAccent => "on-accent".into(),
        zv::Token::Success => "success".into(),
        zv::Token::Warning => "warning".into(),
        zv::Token::Danger => "danger".into(),
        zv::Token::Border => "border".into(),
        zv::Token::Chart(i) => format!("chart{}", i + 1),
        zv::Token::Rgb(rgb) => format!("#{rgb:06x}"),
    }
}

/// A plain token as its name; one mixed or faded as `{ token, mix?, t?, alpha }`.
fn color_value(color: zv::Color) -> Value {
    if color.mix.is_none() && color.alpha >= 1.0 {
        return Value::from(token_name(color.token).as_str());
    }
    let mut fields = vec![("token", Value::from(token_name(color.token).as_str())), ("alpha", num(color.alpha))];
    if let Some((other, t)) = color.mix {
        fields.push(("mix", Value::from(token_name(other).as_str())));
        fields.push(("t", num(t)));
    }
    Value::record(fields)
}

fn align_name(align: zv::Align) -> &'static str {
    match align {
        zv::Align::Start => "start",
        zv::Align::Center => "center",
        zv::Align::End => "end",
    }
}

fn size_name(size: zv::TextSize) -> &'static str {
    match size {
        zv::TextSize::Xs => "xs",
        zv::TextSize::Sm => "sm",
        zv::TextSize::Body => "body",
        zv::TextSize::Lg => "lg",
        zv::TextSize::Xl => "xl",
        zv::TextSize::Xxl => "2xl",
    }
}

fn weight_name(weight: zv::Weight) -> &'static str {
    match weight {
        zv::Weight::Regular => "regular",
        zv::Weight::Medium => "medium",
        zv::Weight::Semibold => "semibold",
        zv::Weight::Bold => "bold",
    }
}

fn format_value(f: &zv::NumberFormat) -> Value {
    let mut fields = vec![("prefix", Value::from(f.prefix.as_str())), ("unit", Value::from(f.unit.as_str()))];
    if let Some(d) = f.decimals {
        fields.push(("decimals", num(d)));
    }
    Value::record(fields)
}

fn handler(id: zv::HandlerId) -> Value {
    num(id.0)
}

fn children_value(children: &[zv::Node], path: &str) -> Value {
    list(children.iter().enumerate().map(|(i, c)| node_value(c, &format!("{path}.{i}"))).collect())
}

/// A node as the scripts get it: `k` its kind, `p` its path in the tree
/// (children by index, a table's cells row by row), and its own fields.
fn node_value(node: &zv::Node, path: &str) -> Value {
    let mut f: Vec<(&str, Value)> = vec![("p", Value::from(path))];
    if let Some(w) = node.width {
        f.push(("w", num(w)));
    }
    if node.grow {
        f.push(("grow", Value::Bool(true)));
    }
    if let Some(key) = &node.key {
        f.push(("key", Value::from(key.as_str())));
    }
    match &node.kind {
        zv::NodeKind::Stack(s) => {
            let kind = if s.axis == zv::Axis::Horizontal { "row" } else { "column" };
            f.push(("k", Value::from(kind)));
            f.push(("gap", num(s.gap)));
            f.push(("align", Value::from(align_name(s.align))));
            f.push((
                "justify",
                Value::from(match s.justify {
                    zv::Justify::Start => "start",
                    zv::Justify::Center => "center",
                    zv::Justify::End => "end",
                    zv::Justify::Between => "between",
                }),
            ));
            f.push(("wrap", Value::Bool(s.wrap)));
            f.push(("hover", Value::Bool(s.hover)));
            if let Some(click) = s.on_click {
                f.push(("click", handler(click)));
            }
            f.push(("children", children_value(&s.children, path)));
        }
        zv::NodeKind::Grid { columns, gap, children } => {
            f.push(("k", Value::from("grid")));
            f.push(("columns", num(*columns)));
            f.push(("gap", num(*gap)));
            f.push(("children", children_value(children, path)));
        }
        zv::NodeKind::Text(t) => {
            f.push(("k", Value::from("text")));
            f.push(("spans", list(t.spans.iter().map(span_value).collect())));
            f.push(("size", Value::from(size_name(t.size))));
            f.push(("weight", Value::from(weight_name(t.weight))));
            if let Some(color) = t.color {
                f.push(("color", color_value(color)));
            }
            f.push(("mono", Value::Bool(t.mono)));
            f.push(("italic", Value::Bool(t.italic)));
            f.push(("align", Value::from(align_name(t.align))));
            f.push(("truncate", Value::Bool(t.truncate)));
        }
        zv::NodeKind::Rule => f.push(("k", Value::from("rule"))),
        zv::NodeKind::Space(size) => {
            f.push(("k", Value::from("space")));
            f.push(("size", num(*size)));
        }
        zv::NodeKind::Stat { value, label, delta, tone } => {
            f.push(("k", Value::from("stat")));
            f.push(("value", Value::from(value.as_str())));
            f.push(("label", Value::from(label.as_str())));
            if let Some(delta) = delta {
                f.push(("delta", Value::from(delta.as_str())));
            }
            if let Some(tone) = tone {
                f.push(("tone", color_value(*tone)));
            }
        }
        zv::NodeKind::Table(t) => {
            f.push(("k", Value::from("table")));
            f.push((
                "columns",
                list(
                    t.columns
                        .iter()
                        .map(|c| {
                            let mut col = vec![("label", Value::from(c.label.as_str())), ("align", Value::from(align_name(c.align)))];
                            if let Some(w) = c.width {
                                col.push(("w", num(w)));
                            }
                            Value::record(col)
                        })
                        .collect(),
                ),
            ));
            // Cells number on from the row before's (`zv::children` reads a
            // table's cells row by row).
            let mut at = 0usize;
            let rows = t
                .rows
                .iter()
                .map(|row| {
                    list(
                        row.iter()
                            .map(|cell| {
                                let v = node_value(cell, &format!("{path}.{at}"));
                                at += 1;
                                v
                            })
                            .collect(),
                    )
                })
                .collect();
            f.push(("rows", list(rows)));
            f.push(("hover", Value::Bool(t.hover)));
        }
        zv::NodeKind::Chart(c) => {
            f.push(("k", Value::from("chart")));
            chart_fields(c, &mut f);
        }
        zv::NodeKind::Canvas(c) => {
            f.push(("k", Value::from("canvas")));
            f.push(("h", num(c.height)));
            f.push(("draw", handler(c.draw)));
            f.push(("animate", Value::Bool(c.animate)));
            if let Some(click) = c.on_click {
                f.push(("click", handler(click)));
            }
        }
        zv::NodeKind::Scene(s) => {
            f.push(("k", Value::from("scene")));
            f.push(("h", num(s.height)));
            f.push(("animate", Value::Bool(s.animate)));
            f.push(("orbit", Value::Bool(s.orbit)));
            f.push(("spin", num(s.spin)));
            f.push(("yaw", num(s.yaw)));
            f.push(("pitch", num(s.pitch)));
            f.push(("zoom", num(s.zoom)));
        }
        zv::NodeKind::Shader(s) => {
            f.push(("k", Value::from("shader")));
            f.push(("h", num(s.height)));
            // (Buffers feed back frame to frame: it runs on.)
            f.push(("animate", Value::Bool(s.animate || !s.buffers.is_empty())));
            f.push(("orbit", Value::Bool(s.orbit)));
            f.push(("spin", num(s.spin)));
            f.push(("yaw", num(s.yaw)));
            f.push(("pitch", num(s.pitch)));
            f.push(("zoom", num(s.zoom)));
        }
        zv::NodeKind::Button { label, on_click, primary } => {
            f.push(("k", Value::from("button")));
            f.push(("label", Value::from(label.as_str())));
            f.push(("click", handler(*on_click)));
            f.push(("primary", Value::Bool(*primary)));
        }
        zv::NodeKind::Tabs { options, value, on_change } => {
            f.push(("k", Value::from("tabs")));
            f.push(("options", list(options.iter().map(|o| Value::from(o.as_str())).collect())));
            f.push(("value", Value::from(value.as_str())));
            f.push(("change", handler(*on_change)));
        }
        zv::NodeKind::Toggle { label, value, on_change } => {
            f.push(("k", Value::from("toggle")));
            f.push(("label", Value::from(label.as_str())));
            f.push(("value", Value::Bool(*value)));
            f.push(("change", handler(*on_change)));
        }
        zv::NodeKind::Slider { label, value, min, max, step, on_change, format } => {
            f.push(("k", Value::from("slider")));
            if let Some(label) = label {
                f.push(("label", Value::from(label.as_str())));
            }
            f.push(("value", num(*value)));
            f.push(("min", num(*min)));
            f.push(("max", num(*max)));
            if *step > 0. {
                f.push(("step", num(*step)));
            }
            f.push(("change", handler(*on_change)));
            f.push(("fmt", format_value(format)));
        }
        zv::NodeKind::Link { label, url } => {
            f.push(("k", Value::from("link")));
            f.push(("label", Value::from(label.as_str())));
            f.push(("url", Value::from(url.as_str())));
        }
    }
    Value::record(f)
}

fn span_value(span: &zv::Span) -> Value {
    let mut f = vec![("t", Value::from(span.text.as_str()))];
    if let Some(weight) = span.weight {
        f.push(("weight", Value::from(weight_name(weight))));
    }
    if let Some(color) = span.color {
        f.push(("color", color_value(color)));
    }
    if span.mono {
        f.push(("mono", Value::Bool(true)));
    }
    if span.italic {
        f.push(("italic", Value::Bool(true)));
    }
    Value::record(f)
}

/// An axis over `lo..hi`: round ticks (crates/visual's `nice_ticks`), the
/// range widened to the outer ones unless the script fixed that end, and
/// each tick's label in the chart's format.
fn axis(mut lo: f64, mut hi: f64, fixed: (Option<f64>, Option<f64>), format: &zv::NumberFormat) -> Value {
    if let Some(min) = fixed.0 {
        lo = min;
    }
    if let Some(max) = fixed.1 {
        hi = max;
    }
    if !lo.is_finite() || !hi.is_finite() {
        (lo, hi) = (0., 1.);
    }
    if (hi - lo).abs() < f64::EPSILON {
        let pad = if lo == 0. { 1. } else { lo.abs() * 0.1 };
        (lo, hi) = (lo - pad, hi + pad);
    }
    let mut ticks: Vec<f64> = zv::nice_ticks(lo, hi, 4);
    // The data's ends inside round ticks: one more step past an end the
    // ticks stop short of (a 190 peak under a 200 line, not over 150).
    if ticks.len() >= 2 {
        let step = ticks[1] - ticks[0];
        if fixed.1.is_none() && ticks[ticks.len() - 1] < hi - step * 1e-6 {
            ticks.push(ticks[ticks.len() - 1] + step);
        }
        if fixed.0.is_none() && ticks[0] > lo + step * 1e-6 {
            ticks.insert(0, ticks[0] - step);
        }
    }
    if fixed.0.is_none()
        && let Some(first) = ticks.first()
    {
        lo = lo.min(*first);
    }
    if fixed.1.is_none()
        && let Some(last) = ticks.last()
    {
        hi = hi.max(*last);
    }
    let shown: Vec<f64> = ticks.into_iter().filter(|t| *t >= lo - 1e-9 && *t <= hi + 1e-9).collect();
    Value::record([
        ("lo", num(lo)),
        ("hi", num(hi)),
        ("ticks", list(shown.iter().map(|t| num(*t)).collect())),
        ("labels", list(shown.iter().map(|t| Value::from(format.format(*t).as_str())).collect())),
    ])
}

fn chart_fields(c: &zv::Chart, f: &mut Vec<(&str, Value)>) {
    let kind = match c.kind {
        zv::ChartKind::Bar => "bar",
        zv::ChartKind::Line => "line",
        zv::ChartKind::Area => "area",
        zv::ChartKind::Scatter => "scatter",
    };
    f.push(("type", Value::from(kind)));
    f.push(("h", num(c.height)));
    f.push(("x", list(c.x.iter().map(|x| Value::from(x.as_str())).collect())));
    f.push(("stacked", Value::Bool(c.stacked)));
    f.push(("horizontal", Value::Bool(c.horizontal && c.kind == zv::ChartKind::Bar)));
    f.push(("legend", Value::Bool(c.legend)));
    f.push(("fmt", format_value(&c.format)));
    f.push(("xfmt", format_value(&c.x_format)));
    f.push((
        "series",
        list(
            c.series
                .iter()
                .map(|s| {
                    let mut points = Vec::with_capacity(s.points.len() * 2);
                    for (x, y) in &s.points {
                        points.push(num(*x));
                        points.push(num(*y));
                    }
                    Value::record([
                        ("name", Value::from(s.name.as_str())),
                        ("color", color_value(s.color)),
                        // (A gap is `false`: a list can't hold nil.)
                        ("values", list(s.values.iter().map(|v| v.map_or(Value::Bool(false), num)).collect())),
                        ("points", list(points)),
                    ])
                })
                .collect(),
        ),
    ));
    // The value axis: from the values (stacked, each category's sums
    // above and below zero), bars and areas from zero.
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    if c.kind == zv::ChartKind::Scatter {
        let (mut xlo, mut xhi) = (f64::INFINITY, f64::NEG_INFINITY);
        for s in &c.series {
            for (x, y) in &s.points {
                (xlo, xhi) = (xlo.min(*x), xhi.max(*x));
                (lo, hi) = (lo.min(*y), hi.max(*y));
            }
        }
        // (Points clear of the plot's edges.)
        let pad = |lo: f64, hi: f64| {
            let d = if (hi - lo).abs() < f64::EPSILON { 1. } else { (hi - lo) * 0.05 };
            (lo - d, hi + d)
        };
        let (xlo, xhi) = pad(xlo, xhi);
        (lo, hi) = pad(lo, hi);
        f.push(("xaxis", axis(xlo, xhi, (None, None), &c.x_format)));
    } else if c.stacked {
        let longest = c.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        for i in 0..longest {
            let (mut up, mut down) = (0f64, 0f64);
            for s in &c.series {
                match s.values.get(i).copied().flatten() {
                    Some(v) if v >= 0. => up += v,
                    Some(v) => down += v,
                    None => {}
                }
            }
            (lo, hi) = (lo.min(down), hi.max(up));
        }
    } else {
        for v in c.series.iter().flat_map(|s| s.values.iter().flatten()) {
            (lo, hi) = (lo.min(*v), hi.max(*v));
        }
    }
    if matches!(c.kind, zv::ChartKind::Bar | zv::ChartKind::Area) {
        (lo, hi) = (lo.min(0.), hi.max(0.));
    }
    f.push(("yaxis", axis(lo, hi, (c.y_min, c.y_max), &c.format)));
}

fn points_value(points: &[[f32; 2]]) -> Value {
    let mut out = Vec::with_capacity(points.len() * 2);
    for [x, y] in points {
        out.push(num(*x));
        out.push(num(*y));
    }
    list(out)
}

/// A canvas command: a stroke (`s`), a fill (`f`) or a line of text (`t`);
/// and the shapes zgx's canvas draws on the GPU as they are, with no path
/// to build or rasterize: a circle (`c`), a rectangle (`r`), a line (`l`).
fn command_value(cmd: &zv::DrawCmd) -> Value {
    match cmd {
        zv::DrawCmd::Fill { points, color } if let Some((x, y, r)) = pixels::circle_of(points) => Value::record([
            ("k", Value::from("c")),
            ("x", num(x)),
            ("y", num(y)),
            ("r", num(r)),
            ("c", color_value(*color)),
        ]),
        zv::DrawCmd::Fill { points, color } if let Some((x, y, w, h)) = pixels::rect_of(points) => Value::record([
            ("k", Value::from("r")),
            ("x", num(x)),
            ("y", num(y)),
            ("w", num(w)),
            ("h", num(h)),
            ("c", color_value(*color)),
        ]),
        zv::DrawCmd::Stroke { points, closed: false, color, width, dash } if points.len() == 2 => {
            let mut f = vec![
                ("k", Value::from("l")),
                ("pts", points_value(points)),
                ("c", color_value(*color)),
                ("w", num(*width)),
            ];
            if let Some(dash) = dash {
                f.push(("dash", num(*dash)));
            }
            Value::record(f)
        }
        // A long solid stroke that turns back (a curve, a spiral): its runs
        // that don't (`m`), each a stroke the canvas draws on the GPU.
        zv::DrawCmd::Stroke { points, closed, color, width, dash: None } if points.len() >= 8 => {
            let mut all = points.clone();
            if *closed {
                all.push(points[0]);
            }
            let runs = pixels::monotone_runs(&all);
            Value::record([
                ("k", Value::from("m")),
                ("runs", list(runs.iter().map(|&(a, b)| points_value(&all[a..=b])).collect())),
                ("c", color_value(*color)),
                ("w", num(*width)),
            ])
        }
        zv::DrawCmd::Stroke { points, closed, color, width, dash } => {
            let mut f = vec![
                ("k", Value::from("s")),
                ("pts", points_value(points)),
                ("closed", Value::Bool(*closed)),
                ("c", color_value(*color)),
                ("w", num(*width)),
            ];
            if let Some(dash) = dash {
                f.push(("dash", num(*dash)));
            }
            Value::record(f)
        }
        zv::DrawCmd::Fill { points, color } => {
            Value::record([("k", Value::from("f")), ("pts", points_value(points)), ("c", color_value(*color))])
        }
        zv::DrawCmd::Text(t) => Value::record([
            ("k", Value::from("t")),
            ("x", num(t.x)),
            ("y", num(t.y)),
            ("t", Value::from(t.text.as_str())),
            ("size", Value::from(size_name(t.size))),
            ("weight", Value::from(weight_name(t.weight))),
            ("c", color_value(t.color)),
            ("mono", Value::Bool(t.mono)),
            ("align", Value::from(align_name(t.align))),
            ("base", Value::from(align_name(t.baseline))),
        ]),
    }
}

// ---- scenes and shaders on the GPU ---------------------------------------------

/// Zeron's pixel helpers (its apps/zgx/pixels, MIT): a canvas's fills read
/// as circles and rectangles, and a polyline's monotone runs.
#[allow(dead_code)]
mod pixels {
    //! Loops over every pixel or point of a frame of something moving (an
    //! agent visual's scene, shader or canvas), optimized even in a debug
    //! build of the app (see Cargo.toml).

    /// Blue-green-red-alpha pixels as red-green-blue-alpha, in place (a frame
    /// read back from crates/visual-gpu, for `zgx::image::ImageData`). A
    /// trailing partial pixel is left as it is.
    pub fn bgra_to_rgba(pixels: &mut [u8]) {
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
    }

    /// A video frame's blue-green-red-unused rows (`pitch` bytes apart, bottom
    /// row first when `bottom_up`) as `w` by `h` opaque red-green-blue-alpha
    /// pixels, into `out` (`w * h * 4` bytes). False if `src` is too short.
    pub fn bgrx_rows_to_rgba(src: &[u8], pitch: usize, bottom_up: bool, w: usize, h: usize, out: &mut [u8]) -> bool {
        let row = w * 4;
        if h == 0 || pitch < row || src.len() < pitch * (h - 1) + row || out.len() < row * h {
            return false;
        }
        for (y, dst) in out.chunks_exact_mut(row).take(h).enumerate() {
            let at = if bottom_up { h - 1 - y } else { y } * pitch;
            for (o, i) in dst.chunks_exact_mut(4).zip(src[at..at + row].chunks_exact(4)) {
                o.copy_from_slice(&[i[2], i[1], i[0], 255]);
            }
        }
        true
    }

    /// A fill crates/visual made for `g:circle`: its centre and radius. Only
    /// its own shape counts (the point count `arc_points` gives that radius, the
    /// first at 3 o'clock, every one as far from the centre), so a polygon the
    /// script drew stays one.
    pub fn circle_of(points: &[[f32; 2]]) -> Option<(f32, f32, f32)> {
        if points.len() < 5 {
            return None;
        }
        let n = points.len() as f32;
        let (cx, cy) = points.iter().fold((0., 0.), |(x, y), p| (x + p[0] / n, y + p[1] / n));
        let r = ((points[0][0] - cx).powi(2) + (points[0][1] - cy).powi(2)).sqrt();
        let segments = ((std::f32::consts::TAU * r.max(1.0)).sqrt() * 2.5).clamp(6.0, 160.0) as usize;
        let starts_east = (points[0][1] - cy).abs() <= r * 0.01 + 0.01 && points[0][0] > cx;
        let round = points.iter().all(|p| (((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt() - r).abs() <= r * 0.01 + 0.01);
        (points.len() == segments && starts_east && round && r > 0.).then_some((cx, cy, r))
    }

    /// A polyline cut where its x turns back, into runs whose x only rises or
    /// only falls (each a stroke zgx's canvas draws on the GPU, not
    /// rasterized): each run's first and last point index, inclusive, the next
    /// run starting where the last ended. Repeated points stay with their run.
    pub fn monotone_runs(points: &[[f32; 2]]) -> Vec<(usize, usize)> {
        let mut runs = Vec::new();
        if points.len() < 2 {
            return runs;
        }
        let (mut start, mut dir) = (0usize, 0i8);
        for i in 1..points.len() {
            let dx = points[i][0] - points[i - 1][0];
            let step = if dx > 0. { 1 } else if dx < 0. { -1 } else { 0 };
            if step == 0 {
                continue;
            }
            if dir == 0 {
                dir = step;
            } else if step != dir {
                runs.push((start, i - 1));
                start = i - 1;
                dir = step;
            }
        }
        runs.push((start, points.len() - 1));
        runs
    }

    /// A fill crates/visual made for a square-cornered `g:rect`: x, y, w, h.
    pub fn rect_of(points: &[[f32; 2]]) -> Option<(f32, f32, f32, f32)> {
        let [a, b, c, d] = points else { return None };
        (a[1] == b[1] && b[0] == c[0] && c[1] == d[1] && d[0] == a[0] && a[0] < b[0] && b[1] < c[1])
            .then(|| (a[0], a[1], b[0] - a[0], c[1] - b[1]))
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn a_square_rect_and_its_corners() {
            let r = [[1., 2.], [5., 2.], [5., 9.], [1., 9.]];
            assert_eq!(super::rect_of(&r), Some((1., 2., 4., 7.)));
            assert_eq!(super::rect_of(&r[..3]), None);
        }

        #[test]
        fn runs_split_where_x_turns_back() {
            let pts = [[0., 0.], [1., 1.], [2., 0.], [1., -1.], [0., 0.], [0., 3.], [1., 4.]];
            assert_eq!(super::monotone_runs(&pts), [(0, 2), (2, 5), (5, 6)], "a vertical step stays in its run");
            assert_eq!(super::monotone_runs(&pts[..3]), [(0, 2)]);
        }

        #[test]
        fn swaps_red_and_blue() {
            let mut px = vec![1, 2, 3, 4, 5, 6, 7, 8];
            super::bgra_to_rgba(&mut px);
            assert_eq!(px, [3, 2, 1, 4, 7, 6, 5, 8]);
        }
    }

}
