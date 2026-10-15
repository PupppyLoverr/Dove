//! Canvas: live, interactive designs an agent builds while it talks — web
//! pages, app screens, mobile mockups, dashboards, React components.
//!
//! An agent calls `show_canvas` (Dove's MCP server) or writes a ` ```html `
//! / ` ```jsx ` fence; Dove serves the page from a loopback server and shows
//! it in the side panel inside a desktop, tablet or phone frame. Pages talk
//! back through `window.dove`: `dove.send(text)` posts a message to the
//! thread (a click on "Use this layout" becomes the user's next turn), and
//! `dove.state(obj)` keeps a value the next `send` carries along.
//!
//! Kinds: `html` (a full document or a fragment), `react` (JSX/TSX with an
//! `App` component; React, hooks and Tailwind are available), and `svg`.
//! React pages load React and Babel from unpkg the first time (cached).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{OnceLock, RwLock};

use dove_core::proto::TaskId;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    px,
};
use gpui_base::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use serde::Deserialize;
use serde_json::Value;

use crate::assets::{icon, icons};
use crate::browser::{BrowserContext, BrowserSurface};
use crate::design::ty;
use crate::kit;
use crate::theme::Theme;
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Html,
    React,
    Svg,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Html => "HTML",
            Kind::React => "React",
            Kind::Svg => "SVG",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Device {
    #[default]
    Desktop,
    Tablet,
    Mobile,
}

impl Device {
    pub const ALL: [Device; 3] = [Device::Desktop, Device::Tablet, Device::Mobile];

    pub fn label(self) -> &'static str {
        match self {
            Device::Desktop => "Desktop",
            Device::Tablet => "Tablet",
            Device::Mobile => "Mobile",
        }
    }

    pub fn parse(id: &str) -> Self {
        match id {
            "tablet" => Device::Tablet,
            "mobile" => Device::Mobile,
            _ => Device::Desktop,
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Device::Desktop => icons::MONITOR,
            Device::Tablet => icons::LAPTOP,
            Device::Mobile => icons::SMARTPHONE,
        }
    }

    /// The viewport a frame gives the page; desktop fills the panel.
    fn viewport(self) -> Option<(f32, f32)> {
        match self {
            Device::Desktop => None,
            Device::Tablet => Some((768.0, 1024.0)),
            Device::Mobile => Some((390.0, 844.0)),
        }
    }
}

/// What an agent sends: one design, by id. Sending the same id again
/// replaces it (iterate on a design in place).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Spec {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default, alias = "html", alias = "source")]
    pub code: String,
    #[serde(default)]
    pub device: Device,
}

impl Spec {
    pub fn from_value(v: Value) -> Option<Self> {
        let mut s: Spec = serde_json::from_value(v).ok()?;
        if s.code.trim().is_empty() {
            return None;
        }
        if s.id.trim().is_empty() {
            s.id = stable_id(&s.title, &s.code);
        }
        if s.title.trim().is_empty() {
            s.title = guess_title(&s.code).unwrap_or_else(|| "Untitled design".into());
        }
        Some(s)
    }

    pub fn parse(src: &str) -> Option<Self> {
        Self::from_value(serde_json::from_str(src).ok()?)
    }

    /// A design from a fenced block (` ```html `, ` ```jsx `, ` ```tsx `, ` ```svg `).
    pub fn from_fence(lang: &str, code: &str) -> Option<Self> {
        let kind = match lang.to_ascii_lowercase().as_str() {
            "html" | "htm" => Kind::Html,
            "jsx" | "tsx" | "react" => Kind::React,
            "ts" | "typescript" | "js" | "javascript"
                if code.contains("function App") || code.contains("export default") =>
            {
                Kind::React
            }
            "svg" => Kind::Svg,
            _ => return None,
        };
        let v = serde_json::json!({ "kind": match kind { Kind::Html => "html", Kind::React => "react", Kind::Svg => "svg" }, "code": code });
        Self::from_value(v)
    }

    pub fn to_json(&self) -> String {
        serde_json::json!({
            "id": self.id,
            "title": self.title,
            "description": self.description,
            "kind": match self.kind { Kind::Html => "html", Kind::React => "react", Kind::Svg => "svg" },
            "code": self.code,
            "device": match self.device { Device::Desktop => "desktop", Device::Tablet => "tablet", Device::Mobile => "mobile" },
        })
        .to_string()
    }

    /// The document the webview loads.
    pub fn page(&self) -> String {
        let bridge = BRIDGE.replace("__ID__", &self.id);
        match self.kind {
            Kind::Html => {
                let code = self.code.trim();
                let lower = code.to_ascii_lowercase();
                if lower.starts_with("<!doctype") || lower.starts_with("<html") {
                    match lower.find("<head>") {
                        Some(i) => format!("{}<head>{bridge}{}", &code[..i], &code[i + 6..]),
                        None => format!("{bridge}{code}"),
                    }
                } else {
                    format!(
                        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" \
                         content=\"width=device-width,initial-scale=1\">{bridge}{BASE_STYLE}\
                         <script src=\"https://cdn.tailwindcss.com\"></script></head><body>{code}</body></html>"
                    )
                }
            }
            Kind::Svg => format!(
                "<!doctype html><html><head><meta charset=\"utf-8\">{bridge}<style>html,body{{margin:0;height:100%;\
                 display:grid;place-items:center;background:#fff}}svg{{max-width:100%;max-height:100vh}}</style></head>\
                 <body>{}</body></html>",
                self.code
            ),
            Kind::React => {
                let code = react_module(&self.code);
                format!(
                    "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" \
                     content=\"width=device-width,initial-scale=1\">{bridge}{BASE_STYLE}\
                     <script src=\"https://cdn.tailwindcss.com\"></script>\
                     <script crossorigin src=\"https://unpkg.com/react@18.3.1/umd/react.production.min.js\"></script>\
                     <script crossorigin src=\"https://unpkg.com/react-dom@18.3.1/umd/react-dom.production.min.js\"></script>\
                     <script src=\"https://unpkg.com/@babel/standalone@7.25.6/babel.min.js\"></script></head>\
                     <body><div id=\"root\"></div><script type=\"text/babel\" data-presets=\"react,typescript\" \
                     data-filename=\"App.tsx\">\n\
                     const {{useState, useEffect, useMemo, useRef, useCallback, useReducer, useContext, createContext, Fragment}} = React;\n\
                     function Icon({{ name = 'circle', size = 16, className = '' }}) {{\n\
                       return React.createElement('span', {{ className: 'inline-flex items-center justify-center ' + className, style: {{ width: size, height: size, fontSize: size * 0.85 }} }}, '•');\n\
                     }}\n\
                     {code}\n\
                     const __Root = typeof App !== 'undefined' ? App : (typeof __default !== 'undefined' ? __default : null);\n\
                     if (__Root) ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(__Root));\n\
                     else document.getElementById('root').textContent = 'Define a component named App.';\n\
                     </script></body></html>"
                )
            }
        }
    }
}

/// Imports dropped (React is global), `export default` made a local.
fn react_module(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let t = line.trim_start();
        if t.starts_with("import ") && (t.contains(" from ") || t.ends_with(';') || t.starts_with("import \"")) {
            continue;
        }
        if let Some(rest) = t.strip_prefix("export default function ") {
            out.push_str(&format!("function {rest}\n"));
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !name.is_empty() && name != "App" {
                out.push_str(&format!("const __default = {name};\n"));
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("export default ") {
            out.push_str(&format!("const __default = {rest}\n"));
            continue;
        }
        if let Some(rest) = t.strip_prefix("export ") {
            out.push_str(rest);
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn stable_id(title: &str, code: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut h);
    code.hash(&mut h);
    format!("c{:x}", h.finish())
}

fn guess_title(code: &str) -> Option<String> {
    let lower = code.to_ascii_lowercase();
    let start = lower.find("<title>")? + 7;
    let end = lower[start..].find("</title>")? + start;
    Some(code[start..end].trim().to_string()).filter(|s| !s.is_empty())
}

const BASE_STYLE: &str = "<style>*,*::before,*::after{box-sizing:border-box}html,body{margin:0}\
body{font-family:ui-sans-serif,system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;-webkit-font-smoothing:antialiased}</style>";

/// `window.dove`: how a page talks back to the thread.
const BRIDGE: &str = "<script>(function(){var s={};window.dove={\
send:function(t){var b=typeof t==='string'?t:JSON.stringify(t);if(Object.keys(s).length)b+='\\n\\nState: '+JSON.stringify(s);\
return fetch('/send/__ID__',{method:'POST',body:b});},\
state:function(o){Object.assign(s,o||{});return s;}};\
document.addEventListener('click',function(e){var a=e.target.closest&&e.target.closest('[data-dove-send]');\
if(a){e.preventDefault();window.dove.send(a.getAttribute('data-dove-send')||a.textContent.trim());}});})();</script>";

// ------------------------------------------------------------ server

struct Doc {
    html: String,
    thread: Option<TaskId>,
}

struct Server {
    port: u16,
    docs: RwLock<HashMap<String, Doc>>,
    inbox: flume::Sender<(Option<TaskId>, String)>,
}

static SERVER: OnceLock<Server> = OnceLock::new();
static INBOX: OnceLock<flume::Receiver<(Option<TaskId>, String)>> = OnceLock::new();

/// Start the loopback server (once). Returns its port.
pub fn start() -> Option<u16> {
    if let Some(s) = SERVER.get() {
        return Some(s.port);
    }
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    let (tx, rx) = flume::unbounded();
    let _ = INBOX.set(rx);
    let _ = SERVER.set(Server { port, docs: RwLock::new(HashMap::new()), inbox: tx });
    std::thread::Builder::new()
        .name("dove-canvas".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    let _ = serve(stream);
                });
            }
        })
        .ok()?;
    Some(port)
}

/// Messages pages sent with `dove.send`, as (thread, text).
pub fn inbox() -> Option<flume::Receiver<(Option<TaskId>, String)>> {
    INBOX.get().cloned()
}

/// Send a design refinement or action back to the thread via the canvas inbox.
pub fn send_to_thread(thread: Option<TaskId>, text: String) {
    let _ = start();
    if let Some(server) = SERVER.get()
        && !text.trim().is_empty()
    {
        let _ = server.inbox.send((thread, text));
    }
}

/// Publish a design; returns the URL that shows it.
pub fn publish(spec: &Spec, thread: Option<TaskId>) -> Option<String> {
    let port = start()?;
    let server = SERVER.get()?;
    if let Ok(mut docs) = server.docs.write() {
        docs.insert(spec.id.clone(), Doc { html: spec.page(), thread });
    }
    // A cache-busting stamp so an updated design reloads.
    let stamp = dove_core::store::now_ms();
    Some(format!("http://127.0.0.1:{port}/c/{}?v={stamp}", spec.id))
}

fn serve(stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut length = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            length = v.trim().parse().unwrap_or(0).min(1 << 20);
        }
    }
    let path = path.split('?').next().unwrap_or_default().to_string();
    let mut out = stream;
    let Some(server) = SERVER.get() else { return Ok(()) };
    if method == "GET"
        && let Some(id) = path.strip_prefix("/c/")
    {
        let html = server.docs.read().ok().and_then(|d| d.get(id).map(|d| d.html.clone()));
        return match html {
            Some(html) => respond(&mut out, "200 OK", "text/html; charset=utf-8", html.as_bytes()),
            None => respond(&mut out, "404 Not Found", "text/plain", b"This design is no longer open."),
        };
    }
    if method == "POST"
        && let Some(id) = path.strip_prefix("/send/")
    {
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body)?;
        let text = String::from_utf8_lossy(&body).trim().to_string();
        let thread = server.docs.read().ok().and_then(|d| d.get(id).and_then(|d| d.thread));
        if !text.is_empty() {
            let _ = server.inbox.send((thread, text));
        }
        return respond(&mut out, "204 No Content", "text/plain", b"");
    }
    respond(&mut out, "404 Not Found", "text/plain", b"Not found")
}

fn respond(out: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        out,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )?;
    out.write_all(body)?;
    out.flush()
}

// ------------------------------------------------------------ chat card

/// The card a design shows in the chat: what it is, and a way to open it.
pub fn card(spec: &Spec, t: &Theme, on_open: impl Fn(&mut Window, &mut gpui::App) + 'static) -> AnyElement {
    let lines = spec.code.lines().count();
    let frame = div()
        .w(px(92.0))
        .h(px(64.0))
        .flex_none()
        .rounded(px(8.0))
        .border_1()
        .border_color(t.border_strong)
        .bg(t.wash(0.04))
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(div().flex().gap(px(3.0)).children([0.28, 0.20, 0.20].map(|a| ui::dot(t.text.opacity(a), 4.0))))
        .child(div().h(px(6.0)).w(px(54.0)).rounded(px(2.0)).bg(t.text.opacity(0.22)))
        .child(div().h(px(4.0)).w(px(70.0)).rounded(px(2.0)).bg(t.text.opacity(0.10)))
        .child(
            div()
                .flex()
                .gap(px(4.0))
                .child(div().h(px(14.0)).flex_1().rounded(px(3.0)).bg(t.accent.opacity(0.35)))
                .child(div().h(px(14.0)).flex_1().rounded(px(3.0)).bg(t.text.opacity(0.08))),
        );
    div()
        .id(SharedString::from(format!("canvas-card-{}", spec.id)))
        .w_full()
        .max_w(px(520.0))
        .p(px(10.0))
        .flex()
        .items_center()
        .gap(px(14.0))
        .rounded(px(14.0))
        .border_1()
        .border_color(t.border)
        .bg(t.wash(0.025))
        .cursor_pointer()
        .hover(|s| s.bg(t.wash(0.05)).border_color(t.border_strong))
        .child(frame)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(
                    div()
                        .text_size(ty::MD)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.text)
                        .truncate()
                        .child(spec.title.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(ty::META)
                        .text_color(t.text_faint)
                        .child(icon(spec.device.glyph()).size(px(12.0)).text_color(t.text_faint))
                        .child(format!("{} · {} · {lines} lines", spec.device.label(), spec.kind.label())),
                )
                .when(!spec.description.trim().is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(ty::SMALL)
                            .text_color(t.text_muted)
                            .line_clamp(2)
                            .child(spec.description.clone()),
                    )
                }),
        )
        .child(
            div()
                .flex_none()
                .h(px(28.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .rounded(px(8.0))
                .bg(t.solid)
                .text_color(t.on_solid)
                .text_size(ty::SMALL)
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(icon(icons::ACTION_PLAY).size(px(13.0)).text_color(t.on_solid))
                .child("Open"),
        )
        .on_click(move |_, window, cx| on_open(window, cx))
        .into_any_element()
}

// ------------------------------------------------------------ viewer

pub enum CanvasEvent {
    Changed,
}

/// One open design: a framed, chromeless webview with device switching,
/// a live code editor, iterative refinement bar, and export.
pub struct CanvasView {
    pub spec: Spec,
    thread: Option<TaskId>,
    browser: Entity<BrowserSurface>,
    device: Device,
    show_code: bool,
    url: Option<String>,
    loaded: bool,
    refine_input: Entity<InputState>,
    code_editor: Entity<TextareaState>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<CanvasEvent> for CanvasView {}

impl CanvasView {
    pub fn new(
        spec: Spec,
        thread: Option<TaskId>,
        ctx: BrowserContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let browser = cx.new(|cx| {
            let mut b = BrowserSurface::new(ctx, false, window, cx);
            b.chromeless = true;
            b
        });
        let refine_input = cx.new(|cx| InputState::new(window, cx).placeholder("Ask agent to refine this design…"));
        let initial_code = spec.code.clone();
        let code_editor = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx).auto_grow(10, 40);
            s.set_value(initial_code, window, cx);
            s
        });
        let sub = cx.subscribe_in(&refine_input, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.submit_refine(window, cx);
            }
        });
        let device = spec.device;
        let mut this = Self {
            spec,
            thread,
            browser,
            device,
            show_code: false,
            url: None,
            loaded: false,
            refine_input,
            code_editor,
            _subs: vec![sub],
        };
        this.load(window, cx);
        this
    }

    pub fn title(&self) -> SharedString {
        self.spec.title.clone().into()
    }

    pub fn browser(&self) -> &Entity<BrowserSurface> {
        &self.browser
    }

    /// Replace the design (same id, new code) and reload it.
    pub fn update_spec(&mut self, spec: Spec, window: &mut Window, cx: &mut Context<Self>) {
        let new_code = spec.code.clone();
        self.code_editor.update(cx, |s, cx| s.set_value(new_code, window, cx));
        self.spec = spec;
        self.load(window, cx);
    }

    fn apply_code_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let edited = self.code_editor.read(cx).value().to_string();
        if !edited.trim().is_empty() {
            self.spec.code = edited;
            self.show_code = false;
            self.load(window, cx);
        }
    }

    fn submit_refine(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.refine_input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.refine_input.update(cx, |s, cx| s.set_value("", window, cx));
        self.send_refinement(&text);
    }

    fn send_refinement(&self, instruction: &str) {
        let msg = format!(
            "Update the Canvas design \"{}\" (id: `{}`) in place via `show_canvas`: {}",
            self.spec.title, self.spec.id, instruction
        );
        send_to_thread(self.thread, msg);
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.url = publish(&self.spec, self.thread);
        if let Some(url) = self.url.clone() {
            self.browser.update(cx, |b, cx| b.navigate_quiet(&url, window, cx));
            self.loaded = true;
        }
        cx.emit(CanvasEvent::Changed);
        cx.notify();
    }

    /// Whether the native page should be live (it hides under the code view).
    pub fn page_visible(&self) -> bool {
        !self.show_code
    }

    fn save_as(&self, cx: &mut Context<Self>) {
        let ext = match self.spec.kind {
            Kind::Html => "html",
            Kind::React => "html",
            Kind::Svg => "svg",
        };
        let name = format!("{}.{ext}", dove_core::git::slug(&self.spec.title));
        let body = if self.spec.kind == Kind::Svg { self.spec.code.clone() } else { self.spec.page() };
        let dir = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ".".into())
            .join("Downloads");
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |_, _| {
            if let Ok(Ok(Some(path))) = rx.await {
                let _ = std::fs::write(path, body);
            }
        })
        .detach();
    }
}

impl Render for CanvasView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("CanvasView");
        let t = Theme::of(cx).clone();
        let device = self.device;
        let devices = div().flex().items_center().gap(px(2.0)).p(px(2.0)).rounded(px(8.0)).bg(t.wash(0.05)).children(
            Device::ALL.into_iter().map(|d| {
                let on = d == device && !self.show_code;
                div()
                    .id(SharedString::from(format!("canvas-dev-{}", d.label())))
                    .size(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .tooltip(ui::tip(d.label()))
                    .when(on, |s| s.bg(t.surface_raised).shadow_sm())
                    .hover(|s| s.bg(t.wash(0.08)))
                    .child(icon(d.glyph()).size(px(14.0)).text_color(if on { t.text } else { t.text_faint }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.device = d;
                        this.show_code = false;
                        cx.notify();
                    }))
            }),
        );
        let tool = |id: &'static str, glyph: &'static str, tip: &'static str| {
            div()
                .id(id)
                .size(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(7.0))
                .cursor_pointer()
                .tooltip(ui::tip(tip))
                .hover(|s| s.bg(t.wash(0.08)))
                .child(icon(glyph).size(px(15.0)).text_color(t.text_muted))
        };
        let toolbar = div()
            .h(px(44.0))
            .flex_none()
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .border_b_1()
            .border_color(t.border)
            .child(icon(icons::GALLERY).size(px(15.0)).text_color(t.text_faint))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(ty::BODY).text_color(t.text).truncate().child(self.spec.title.clone()))
                    .child(div().text_size(ty::MICRO).text_color(t.text_faint).child(format!(
                        "{} · {} · live",
                        self.spec.kind.label(),
                        device.label()
                    ))),
            )
            .child(devices)
            .child(
                tool("canvas-code", icons::FILE_CODE, "Edit source code")
                    .when(self.show_code, |d| d.bg(t.wash(0.10)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_code = !this.show_code;
                        cx.notify();
                    })),
            )
            .child(tool("canvas-reload", icons::REFRESH, "Reload").on_click(cx.listener(|this, _, window, cx| {
                this.load(window, cx);
            })))
            .child(tool("canvas-copy", icons::COPY, "Copy code").on_click(cx.listener(|this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(this.spec.code.clone()));
                kit::Toasts::push(cx, "Code copied", None, kit::Tone::Success, None);
            })))
            .child(tool("canvas-save", icons::FLOPPY_DISK, "Save as file").on_click(cx.listener(|this, _, _, cx| {
                this.save_as(cx);
            })))
            .child(tool("canvas-open", icons::ARROW_UP_RIGHT, "Open in your browser").on_click(cx.listener(
                |this, _, _, cx| {
                    if let Some(url) = &this.url {
                        cx.open_url(url);
                    }
                },
            )));
        let body: AnyElement = if self.show_code {
            div()
                .id("canvas-code-view")
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(36.0))
                        .px(px(12.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .border_b_1()
                        .border_color(t.border)
                        .bg(t.wash(0.025))
                        .text_size(ty::META)
                        .text_color(t.text_muted)
                        .child(format!("{} source ({} lines)", self.spec.kind.label(), self.spec.code.lines().count()))
                        .child(div().flex_1())
                        .child(
                            ui::button_primary("canvas-apply-code", "Apply & Reload", &t)
                                .h(px(24.0))
                                .text_size(ty::CAPTION)
                                .on_click(cx.listener(|this, _, window, cx| this.apply_code_edit(window, cx))),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .p(px(12.0))
                        .font_family(t.font_mono.clone())
                        .text_size(ty::SMALL)
                        .child(Textarea::new(&self.code_editor)),
                )
                .into_any_element()
        } else {
            let page = div().size_full().child(self.browser.clone());
            match device.viewport() {
                None => page.into_any_element(),
                Some((w, h)) => {
                    let mobile = device == Device::Mobile;
                    let bezel = if mobile { 12.0 } else { 14.0 };
                    let radius = if mobile { 44.0 } else { 26.0 };
                    let avail = window.viewport_size();
                    let max_h = (f32::from(avail.height) - 180.0).max(320.0);
                    let scale = (max_h / (h + bezel * 2.0)).min(1.0);
                    let (fw, fh) = ((w + bezel * 2.0) * scale, (h + bezel * 2.0) * scale);
                    div()
                        .id("canvas-stage")
                        .size_full()
                        .overflow_scroll()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(t.wash(0.03))
                        .p(px(20.0))
                        .child(
                            div()
                                .w(px(fw))
                                .h(px(fh))
                                .flex_none()
                                .p(px(bezel * scale))
                                .rounded(px(radius * scale))
                                .bg(gpui::hsla(0.0, 0.0, 0.08, 1.0))
                                .border_1()
                                .border_color(gpui::hsla(0.0, 0.0, 1.0, 0.10))
                                .shadow_lg()
                                .child(ui::rounded_clip(
                                    (radius - bezel) * scale,
                                    div()
                                        .size_full()
                                        .rounded(px((radius - bezel) * scale))
                                        .overflow_hidden()
                                        .child(page),
                                )),
                        )
                        .into_any_element()
                }
            }
        };

        let quick_refines: &[(&str, &str)] = &[
            ("Dark mode", "Convert this design to a sleek, high-contrast dark theme with subtle borders."),
            ("Mobile polish", "Optimize this layout for a mobile phone viewport with a bottom navigation bar."),
            ("Add interactions", "Add interactive state, tabs, filtering, and realistic sample data."),
            ("Minimalist pass", "Strip visual clutter, tighten typography hierarchy, and use crisp neutral surfaces."),
        ];
        let footer = div()
            .flex_none()
            .px(px(10.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(t.border)
            .bg(t.wash(0.02))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(div().flex().flex_wrap().gap(px(5.0)).children(quick_refines.iter().enumerate().map(
                |(i, (chip, prompt))| {
                    let prompt = (*prompt).to_string();
                    div()
                        .id(SharedString::from(format!("canvas-qref-{i}")))
                        .h(px(22.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .rounded(px(5.0))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.wash(0.03))
                        .cursor_pointer()
                        .text_size(ty::MICRO)
                        .text_color(t.text_muted)
                        .hover(|s| s.bg(t.wash(0.07)).text_color(t.text))
                        .child(*chip)
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.send_refinement(&prompt);
                        }))
                },
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_1()
                            .h(px(28.0))
                            .px(px(8.0))
                            .flex()
                            .items_center()
                            .rounded(px(7.0))
                            .border_1()
                            .border_color(t.border)
                            .bg(t.wash(0.04))
                            .text_size(ty::SMALL)
                            .child(Input::new(&self.refine_input)),
                    )
                    .child(
                        ui::button_primary("canvas-refine-send", "Refine", &t)
                            .h(px(28.0))
                            .on_click(cx.listener(|this, _, window, cx| this.submit_refine(window, cx))),
                    ),
            );

        div()
            .id("canvas-view")
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(body))
            .child(footer)
    }
}

/// Sizes of the page (for tests and the tool description).
#[allow(dead_code)]
pub fn viewport_of(d: Device) -> Option<(f32, f32)> {
    d.viewport()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_get_a_document_and_the_bridge() {
        let s = Spec::from_fence("html", "<h1>Hi</h1>").unwrap();
        let page = s.page();
        assert!(page.starts_with("<!doctype html>"));
        assert!(page.contains("window.dove"));
        assert!(page.contains("<h1>Hi</h1>"));
    }

    #[test]
    fn full_documents_keep_their_head() {
        let s = Spec::from_fence("html", "<!doctype html><html><head><title>Shop</title></head><body></body></html>")
            .unwrap();
        assert_eq!(s.title, "Shop");
        assert!(s.page().contains("<head><script>(function(){var s={};window.dove"));
    }

    #[test]
    fn react_exports_become_locals() {
        let out = react_module("import React from 'react';\nexport default function Card() { return null }\n");
        assert!(!out.contains("import"));
        assert!(out.contains("function Card()"));
        assert!(out.contains("const __default = Card;"));
    }

    #[test]
    fn same_code_same_id() {
        let a = Spec::from_fence("svg", "<svg/>").unwrap();
        let b = Spec::from_fence("svg", "<svg/>").unwrap();
        assert_eq!(a.id, b.id);
    }
}
