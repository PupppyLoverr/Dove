//! The cua-driver backend (Hermes `cua_backend*.py`): window discovery,
//! capture, and every input action against the sticky target, over one
//! MCP connection to stock `cua-driver mcp`.

use std::collections::BTreeMap;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::driver::{Driver, Envelope, Schemas};
use crate::parse::{self, Window};
use crate::types::{ActionResult, Capture, UiElement};

/// `app=` values that mean "the whole screen" (pixels only).
const FULL_SCREEN: &[&str] = &["screen", "fullscreen", "full screen", "all"];
/// Shell windows: Windows desktop and taskbar, macOS Finder desktop and Dock.
const DESKTOP_NAMES: &[&str] = &["progman", "workerw", "program manager", "shell_traywnd", "taskbar", "finder", "desktop", "dock"];
const DESKTOP_BACKDROP: &[&str] = &["progman", "workerw", "program manager", "finder", "desktop"];
/// Bound on the driver's accessibility walk: the response shows 100 rows,
/// so walking thousands of nodes only costs latency.
pub const AX_MAX_ELEMENTS: u32 = 250;
/// Longest screenshot edge: legible element labels at a fraction of the tokens.
pub const MAX_IMAGE_DIMENSION: u32 = 1456;
/// A tree with fewer actionable rows than this is sparse: canvas, game,
/// remote desktop, custom-drawn UI. Auto mode adds pixels then.
pub const SPARSE_TREE: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub pid: u32,
    pub window_id: u64,
    pub app: String,
    pub title: String,
}

/// What `capture` is asked for.
#[derive(Clone, Debug, Default)]
pub struct CaptureRequest {
    pub mode: String,
    pub app: Option<String>,
    pub pid: Option<u32>,
    pub window_id: Option<u64>,
    pub query: Option<String>,
    pub since: Option<String>,
    pub max_elements: Option<u32>,
}

pub struct Cua<D: Driver> {
    pub driver: D,
    schemas: Schemas,
    session: String,
    started: bool,
    pub target: Option<Target>,
    /// The last capture's rows, by index, and the raw driver rows behind
    /// them (the grounder reads those).
    pub elements: BTreeMap<u32, UiElement>,
    pub raw_rows: BTreeMap<u32, Value>,
    pub snapshot: Option<String>,
    /// The query/max_elements the snapshot was read with (`since` needs the same).
    view: (Option<String>, Option<u32>),
    /// The driver's latest read of the target carried a screenshot: pixel
    /// coordinates are admitted against that frame, so a text-only read
    /// disarms them until a screenshot read re-arms.
    framed: bool,
}

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

impl<D: Driver> Cua<D> {
    pub fn new(driver: D, session: String) -> Self {
        Self {
            driver,
            schemas: Schemas::default(),
            session,
            started: false,
            target: None,
            elements: BTreeMap::new(),
            raw_rows: BTreeMap::new(),
            snapshot: None,
            view: (None, None),
            framed: false,
        }
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    /// Read the live schemas and declare this run's session (non-fatal:
    /// the driver also accepts anonymous calls).
    pub fn start(&mut self) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        let tools = self.driver.list_tools()?;
        if tools.is_empty() {
            return Err("cua-driver listed no tools".into());
        }
        self.schemas = Schemas::from_tools(&tools);
        self.started = true;
        if self.schemas.has_tool("start_session") {
            let _ = self.call("start_session", Map::new());
        }
        Ok(())
    }

    pub fn has_tool(&self, tool: &str) -> bool {
        self.schemas.has_tool(tool)
    }

    pub fn accepts(&self, tool: &str, prop: &str) -> bool {
        self.schemas.accepts(tool, prop)
    }

    /// One driver call: `session` added where accepted, then every property
    /// the live schema would refuse dropped.
    pub fn call(&mut self, tool: &str, mut args: Map<String, Value>) -> Result<Envelope, String> {
        if self.schemas.accepts(tool, "session") && !args.contains_key("session") {
            args.insert("session".into(), json!(self.session));
        }
        let args = self.schemas.filter(tool, args);
        self.driver.call_tool(tool, Value::Object(args)).map(|r| Envelope::from_result(&r))
    }

    pub fn end(&mut self) {
        if self.started && self.schemas.has_tool("end_session") {
            let _ = self.call("end_session", Map::new());
        }
    }

    fn forget_target(&mut self) {
        self.target = None;
        self.elements.clear();
        self.raw_rows.clear();
        self.snapshot = None;
    }

    // ------------------------------------------------------------ windows

    pub fn list_windows(&mut self) -> Result<Vec<Window>, String> {
        self.windows(true)
    }

    /// `list_windows`; `on_screen_only: false` adds minimized and hidden
    /// windows, which background input still reaches.
    pub fn windows(&mut self, on_screen_only: bool) -> Result<Vec<Window>, String> {
        let env = self.call("list_windows", obj(json!({"on_screen_only": on_screen_only})))?;
        if env.is_error {
            return Err(format!("list_windows failed: {}", env.message()));
        }
        Ok(parse::windows(&env))
    }

    pub fn list_apps(&mut self) -> Result<Vec<Value>, String> {
        let env = self.call("list_apps", Map::new())?;
        if env.is_error {
            return Err(format!("list_apps failed: {}", env.message()));
        }
        let apps = parse::apps(&env);
        if !apps.is_empty() {
            return Ok(apps);
        }
        // Old text-only drivers: "Name (pid 123)" lines.
        let re = Regex::new(r"(.+?)\s+\(pid\s+(\d+)\)").expect("apps regex");
        Ok(env.text().lines().filter_map(|l| re.captures(l)).map(|c| json!({"name": c[1].trim(), "pid": c[2].parse::<u64>().unwrap_or(0)})).collect())
    }

    /// Windows of `app`, cheapest evidence first: the process name without
    /// `.exe`, then titles of windows whose process only hosts them (UWP
    /// apps live in ApplicationFrameHost), then name substrings, then
    /// `list_apps` aliases (bundle ids, display names; slow on Windows, where
    /// it lists installed apps too), then titles of nameless rows. Asking
    /// for `Code` must not pick `Visual Studio Code` by z-order.
    pub fn match_app(&mut self, windows: &[Window], app: &str) -> Vec<Window> {
        let want = norm_app(app);
        if want.is_empty() {
            return Vec::new();
        }
        let pick = |f: &dyn Fn(&Window) -> bool| -> Vec<Window> { windows.iter().filter(|w| f(w)).cloned().collect() };
        let title = |w: &Window| w.title.trim().to_lowercase();
        let tiers = [
            pick(&|w| norm_app(&w.app) == want),
            pick(&|w| is_host(&w.app) && title(w) == want),
            pick(&|w| is_host(&w.app) && title(w).contains(&want)),
            pick(&|w| !is_host(&w.app) && norm_app(&w.app).contains(&want)),
        ];
        if let Some(t) = tiers.into_iter().find(|t| !t.is_empty()) {
            return t;
        }
        let apps = self.list_apps().unwrap_or_default();
        let (mut exact_pids, mut partial_pids) = (Vec::new(), Vec::new());
        for a in &apps {
            let Some(pid) = a.get("pid").and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok()).filter(|p| *p > 0) else {
                continue;
            };
            if a.get("running") == Some(&Value::Bool(false)) {
                continue;
            }
            let aliases: Vec<String> = ["bundle_id", "bundleId", "name", "app_name", "display_name", "aumid"]
                .iter()
                .filter_map(|k| a.get(*k).and_then(Value::as_str))
                .map(norm_app)
                .filter(|s| !s.is_empty())
                .collect();
            if aliases.contains(&want) {
                exact_pids.push(pid);
            } else if aliases.iter().any(|al| al.contains(&want)) {
                partial_pids.push(pid);
            }
        }
        let tiers = [
            pick(&|w| exact_pids.contains(&w.pid)),
            pick(&|w| partial_pids.contains(&w.pid)),
            pick(&|w| w.app.trim().is_empty() && title(w).contains(&want)),
        ];
        tiers.into_iter().find(|t| !t.is_empty()).unwrap_or_default()
    }

    /// The best window of a frontmost-first list. Implicit picks skip the
    /// driver's own window; on Linux they skip shell helpers and, when
    /// every window shares one z-order, prefer `_NET_ACTIVE_WINDOW`.
    fn select(windows: &[Window], app_requested: bool) -> Option<Window> {
        let mut pool: Vec<&Window> = windows.iter().filter(|w| !w.off_screen).collect();
        let external: Vec<&Window> = pool.iter().copied().filter(|w| !parse::is_driver_window(w)).collect();
        if !external.is_empty() {
            pool = external;
        }
        if !app_requested && cfg!(target_os = "linux") {
            let real: Vec<&Window> = pool.iter().copied().filter(|w| parse::is_real_app_window(w)).collect();
            if !real.is_empty() {
                pool = real;
            }
            let flat = pool.iter().map(|w| w.z.to_bits()).collect::<std::collections::HashSet<_>>().len() <= 1;
            if flat && let Some(active) = linux_active_window() && let Some(w) = pool.iter().find(|w| w.window_id == active) {
                return Some((*w).clone());
            }
        }
        pool.first().map(|w| (*w).clone()).or_else(|| windows.first().cloned())
    }

    /// Point input at `app`'s frontmost window without raising it.
    pub fn focus_app(&mut self, app: &str) -> ActionResult {
        let windows = match self.list_windows() {
            Ok(w) => w,
            Err(e) => {
                self.forget_target();
                return ActionResult::refused("focus_app", e, None);
            }
        };
        let matched = self.match_app(&windows, app);
        let Some(w) = matched.first().cloned() else {
            self.forget_target();
            return ActionResult::refused("focus_app", format!("No on-screen window found for app {app:?}. Call list_apps or list_windows to see what is open."), Some("no_window"));
        };
        self.set_target(&w);
        ActionResult {
            ok: true,
            action: "focus_app".into(),
            message: format!("Targeted {} (pid {}, window {}) without raising it.", w.app, w.pid, w.window_id),
            ..ActionResult::default()
        }
    }

    fn set_target(&mut self, w: &Window) {
        if self.target.as_ref().map(|t| (t.pid, t.window_id)) != Some((w.pid, w.window_id)) {
            self.framed = false;
            self.elements.clear();
            self.raw_rows.clear();
            self.snapshot = None;
        }
        self.target = Some(Target { pid: w.pid, window_id: w.window_id, app: w.app.clone(), title: w.title.clone() });
    }

    /// Raise the target (a visible focus change, approved separately).
    pub fn bring_to_front(&mut self) -> ActionResult {
        let Some(t) = self.target.clone() else {
            return ActionResult::refused("bring_to_front", "Capture an exact target first.", Some("no_target"));
        };
        if !self.schemas.has_tool("bring_to_front") {
            return ActionResult::refused("bring_to_front", "This cua-driver has no bring_to_front tool.", Some("bring_to_front_unsupported"));
        }
        self.action("bring_to_front", obj(json!({"pid": t.pid, "window_id": t.window_id})), None)
    }

    // ------------------------------------------------------------ capture

    /// Capture: resolve the window, then read it.
    pub fn capture(&mut self, req: &CaptureRequest) -> Capture {
        match self.resolve(req) {
            Ok(Some(w)) => self.read_target(&w, req),
            Ok(None) => self.capture_screen(if req.mode.is_empty() { "auto" } else { &req.mode }),
            Err(cap) => cap,
        }
    }

    /// The window a capture names: Ok(None) for the composited screen, a
    /// failed capture (with the reason) when nothing matches.
    pub fn resolve(&mut self, req: &CaptureRequest) -> Result<Option<Window>, Capture> {
        let mode = if req.mode.is_empty() { "auto".to_string() } else { req.mode.clone() };
        let exact = req.pid.is_some() || req.window_id.is_some();
        let app = req.app.as_deref().map(str::trim).filter(|a| !a.is_empty());
        if !exact && let Some(a) = app && FULL_SCREEN.contains(&a.to_lowercase().as_str()) {
            return Ok(None);
        }
        if exact {
            let (Some(pid), Some(window_id)) = (req.pid, req.window_id) else {
                return self.failed(&mode, "capture targeting needs both pid and window_id".into());
            };
            // Re-reads of the current target skip discovery (one driver
            // round trip less); a new exact target is named from it.
            if let Some(t) = self.target.as_ref().filter(|t| t.pid == pid && t.window_id == window_id) {
                return Ok(Some(Window { app: t.app.clone(), pid, window_id, title: t.title.clone(), off_screen: false, z: 0.0 }));
            }
            let known = self.list_windows().ok().and_then(|ws| ws.into_iter().find(|w| w.pid == pid && w.window_id == window_id));
            return Ok(Some(known.unwrap_or(Window {
                app: app.unwrap_or("").to_string(),
                pid,
                window_id,
                title: String::new(),
                off_screen: false,
                z: 0.0,
            })));
        }
        let windows = match self.list_windows() {
            Ok(w) if !w.is_empty() => w,
            Ok(_) => return self.failed(&mode, empty_discovery_reason()),
            Err(e) => return self.failed(&mode, e),
        };
        let pool = match app {
            None => windows,
            Some(a) if a.eq_ignore_ascii_case("desktop") => {
                let mut d: Vec<Window> = windows.into_iter().filter(|w| named(w, DESKTOP_NAMES)).collect();
                d.sort_by_key(|w| !named(w, DESKTOP_BACKDROP));
                if d.is_empty() {
                    return self.failed(&mode, "no desktop or taskbar window found; capture a specific app instead (on Windows the taskbar is Shell_TrayWnd and the desktop is Progman)".into());
                }
                d
            }
            Some(a) => {
                let mut m = self.match_app(&windows, a);
                if m.is_empty() {
                    // Minimized: not on screen, but background input reaches it.
                    let all = self.windows(false).unwrap_or_default();
                    m = self.match_app(&all, a).into_iter().filter(|w| !parse::is_driver_window(w)).collect();
                    for w in &mut m {
                        w.off_screen = false;
                    }
                }
                if m.is_empty() {
                    return self.failed(&mode, format!("no window matched app={a:?}; call list_apps for the names and bundle ids (macOS reports localized names)"));
                }
                m
            }
        };
        match Self::select(&pool, app.is_some()) {
            Some(w) => Ok(Some(w)),
            None => self.failed(&mode, "no window to capture".into()),
        }
    }

    fn failed(&mut self, mode: &str, note: String) -> Result<Option<Window>, Capture> {
        self.forget_target();
        Err(Capture { mode: mode.to_string(), note, ..Capture::default() })
    }

    /// Read a resolved window and make it the sticky target.
    pub fn read_target(&mut self, target: &Window, req: &CaptureRequest) -> Capture {
        let mode = if req.mode.is_empty() { "auto".to_string() } else { req.mode.clone() };
        self.set_target(target);
        // OCR rows belong to the frame they were read from.
        self.elements.retain(|_, e| e.source == "tree");
        let mut cap = self.read_window(&mode, req);
        if cap.app.is_empty() {
            cap.app = target.app.clone();
        }
        if cap.window_title.is_empty() {
            cap.window_title = target.title.clone();
        }
        cap.pid = Some(target.pid);
        cap.window_id = Some(target.window_id);
        cap
    }

    fn gws_args(&self, req: &CaptureRequest, screenshot: bool, tree: bool) -> Map<String, Value> {
        let t = self.target.as_ref().expect("target set before read");
        let mut a = obj(json!({
            "pid": t.pid, "window_id": t.window_id, "tree_format": "elements",
            "max_elements": req.max_elements.unwrap_or(AX_MAX_ELEMENTS),
            "max_image_dimension": MAX_IMAGE_DIMENSION,
            "include_screenshot": screenshot,
            "include_accessibility_tree": tree,
        }));
        if let Some(q) = req.query.as_deref().filter(|q| !q.trim().is_empty()) {
            a.insert("query".into(), json!(q));
        }
        a
    }

    fn read_window(&mut self, mode: &str, req: &CaptureRequest) -> Capture {
        let (screenshot, tree) = match mode {
            "vision" => (true, false),
            "ax" | "auto" => (false, true),
            _ => (true, true),
        };
        let mut args = self.gws_args(req, screenshot, tree);
        let view = (req.query.clone().filter(|q| !q.trim().is_empty()), req.max_elements);
        // `since` compares against a read of the same view only.
        let since = req.since.clone().filter(|_| tree && self.snapshot.is_some() && self.view == view);
        if let Some(s) = &since {
            args.insert("since".into(), json!(s));
            // cua-driver 0.38 fails its own output check on a `since` read in
            // the `elements` format; the markdown diff is what the model
            // reads anyway, and a change is followed by a full read below.
            args.insert("tree_format".into(), json!("markdown"));
        }
        let env = match self.call("get_window_state", args) {
            Ok(e) if !e.is_error => e,
            Ok(e) => {
                let note = format!("get_window_state failed: {}", e.message());
                self.forget_target();
                return Capture { mode: mode.into(), note, ..Capture::default() };
            }
            Err(e) => {
                self.forget_target();
                return Capture { mode: mode.into(), note: format!("cua-driver error: {e}"), ..Capture::default() };
            }
        };
        self.framed = env.image().is_some();
        let mut cap = Capture { mode: mode.into(), ..Capture::default() };
        let s = |k: &str| env.field(k).and_then(Value::as_str).unwrap_or("").to_string();
        cap.app = s("app_name");
        cap.window_title = s("window_title");
        cap.truncated = env.field("truncated").and_then(Value::as_bool).unwrap_or(false);
        let sid = env.field("snapshot_id").and_then(Value::as_str).map(str::to_string);
        if tree {
            let diff = env.field("tree_diff").and_then(Value::as_str).map(str::to_string);
            let status = s("since_status");
            match diff.filter(|_| since.is_some() && (status == "diff" || status == "no_change")) {
                Some(text) if status == "no_change" => {
                    // Same rows, new snapshot: re-mint the tokens.
                    self.apply_diff(&env, &text, sid.as_deref());
                    cap.diff = Some(text);
                }
                Some(text) => {
                    // Something changed: a full read keeps every row, frame
                    // and index exact; the model still reads only the diff.
                    let full = CaptureRequest { since: None, ..req.clone() };
                    let mut again = self.read_window(mode, &full);
                    again.diff = Some(text);
                    return again;
                }
                None => {
                    self.replace_rows(&env, sid.as_deref());
                    if self.elements.values().all(is_chrome) {
                        self.markdown_rows(req);
                    }
                }
            }
            self.view = view;
            cap.elements = self.elements.values().cloned().collect();
        }
        cap.snapshot_id = self.snapshot.clone();
        // Windows parks minimized windows near (-32000, -32000): their rows
        // still act by index, but no coordinate or screenshot means anything.
        let parked = cap.elements.iter().filter(|e| e.bounds_known() && e.bounds.0 < -10_000 && e.bounds.1 < -10_000).count();
        if parked > 0 && parked * 2 >= cap.elements.iter().filter(|e| e.bounds_known()).count() {
            cap.note = "the window is minimized: act by element index (background input still reaches it); coordinates and screenshots mean nothing until it is restored".into();
            for e in self.elements.values_mut() {
                e.bounds = (0, 0, 0, 0);
            }
            cap.elements = self.elements.values().cloned().collect();
            return cap;
        }
        if let Some((img, mime)) = env.image() {
            cap.image = Some(img);
            cap.image_mime = mime;
        }
        // Window chrome does not make a tree rich: a XAML app whose content
        // is missing from the tree (modern Notepad, Calculator at times)
        // still shows its title bar and caption buttons.
        let actionable = cap
            .elements
            .iter()
            .filter(|e| e.enabled && (e.bounds_known() || e.token.is_some()) && !is_chrome(e))
            .count();
        let degraded = env.field("degraded").and_then(Value::as_bool).unwrap_or(false);
        if mode == "auto" && cap.image.is_none() && cap.diff.is_none() && (actionable < SPARSE_TREE || degraded) {
            // Canvas, game, remote desktop: the tree cannot carry it alone.
            if let Ok(shot) = self.call("get_window_state", self.gws_args(req, true, false))
                && !shot.is_error
                && let Some((img, mime)) = shot.image()
            {
                self.framed = true;
                cap.image = Some(img);
                cap.image_mime = mime;
                cap.image_reason = Some("the accessibility tree is sparse here, so a screenshot is attached; act on what you see by coordinate, or on the OCR rows");
            }
        }
        cap
    }

    fn replace_rows(&mut self, env: &Envelope, sid: Option<&str>) {
        self.elements.clear();
        self.raw_rows.clear();
        let rows = env.structured.get("elements").and_then(Value::as_array).cloned().unwrap_or_default();
        for e in parse::elements(env) {
            self.elements.insert(e.index, e);
        }
        for r in rows {
            if let Some(i) = r.get("element_index").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok()) {
                self.raw_rows.insert(i, r);
            }
        }
        self.snapshot = sid.map(str::to_string).or_else(|| {
            self.elements.values().find_map(|e| e.token.as_deref()?.split_once(':').map(|(s, _)| s.to_string()))
        });
    }

    /// cua-driver 0.38's `elements` walk can start at a window's title bar
    /// and return only its chrome (PowerPoint, after a while), while the
    /// markdown walk of the same window still has every row. Take those.
    fn markdown_rows(&mut self, req: &CaptureRequest) {
        let mut args = self.gws_args(req, false, true);
        args.insert("tree_format".into(), json!("markdown"));
        let Ok(env) = self.call("get_window_state", args) else { return };
        let sid = env.field("snapshot_id").and_then(Value::as_str).map(str::to_string);
        let (Some(text), Some(sid)) = (env.field("tree_markdown").and_then(Value::as_str), sid) else { return };
        let rows = parse::markdown_elements(text, &sid);
        if env.is_error || rows.iter().all(is_chrome) {
            return;
        }
        self.elements.clear();
        self.raw_rows.clear();
        for e in rows {
            let raw = json!({"element_index": e.index, "element_token": e.token, "role": e.role, "label": e.label, "value": e.value});
            self.raw_rows.insert(e.index, raw);
            self.elements.insert(e.index, e);
        }
        self.snapshot = Some(sid);
    }

    /// Merge a `since` diff: changed and added rows replace theirs, removed
    /// rows go, and every token is re-minted against the new snapshot.
    fn apply_diff(&mut self, env: &Envelope, text: &str, sid: Option<&str>) {
        let removed = Regex::new(r"(?m)^-\s*\[(\d+)\]").expect("diff regex");
        for c in removed.captures_iter(text) {
            if let Ok(i) = c[1].parse::<u32>() {
                self.elements.remove(&i);
                self.raw_rows.remove(&i);
            }
        }
        for e in parse::elements(env) {
            self.elements.insert(e.index, e);
        }
        for r in env.structured.get("elements").and_then(Value::as_array).cloned().unwrap_or_default() {
            if let Some(i) = r.get("element_index").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok()) {
                self.raw_rows.insert(i, r);
            }
        }
        if let Some(sid) = sid {
            self.snapshot = Some(sid.to_string());
            for (i, e) in self.elements.iter_mut() {
                e.token = Some(format!("{sid}:{i}"));
            }
            for (i, r) in self.raw_rows.iter_mut() {
                r["element_token"] = json!(format!("{sid}:{i}"));
            }
        }
    }

    /// The composited screen (pixels only; nothing to click by index).
    fn capture_screen(&mut self, mode: &str) -> Capture {
        self.forget_target();
        let env = match self.call("get_desktop_state", obj(json!({"max_image_dimension": MAX_IMAGE_DIMENSION}))) {
            Ok(e) if !e.is_error => e,
            Ok(e) => return Capture { mode: mode.into(), note: format!("get_desktop_state failed: {}", e.message()), ..Capture::default() },
            Err(e) => return Capture { mode: mode.into(), note: format!("cua-driver error: {e}"), ..Capture::default() },
        };
        let Some((img, mime)) = env.image() else {
            return Capture { mode: mode.into(), note: "get_desktop_state returned no image; capture a specific app instead".into(), ..Capture::default() };
        };
        Capture {
            mode: "vision".into(),
            image: Some(img),
            image_mime: mime,
            app: "screen".into(),
            window_title: "Full screen (composited)".into(),
            note: "a full-screen capture has no elements; capture(app='<App>') for that app's clickable rows, or capture(app='desktop') for the desktop and taskbar".into(),
            ..Capture::default()
        }
    }

    // ------------------------------------------------------------ input

    pub fn element(&self, index: u32) -> Option<&UiElement> {
        self.elements.get(&index)
    }

    fn need_target(&self, action: &str) -> Result<Target, ActionResult> {
        self.target.clone().ok_or_else(|| ActionResult::refused(action, "No target window: call capture (or focus_app) first.", Some("no_target")))
    }

    /// Element addressing: its token, or (OCR rows) its center by pixel.
    fn address(&self, action: &str, index: u32) -> Result<Map<String, Value>, ActionResult> {
        let t = self.need_target(action)?;
        let Some(e) = self.elements.get(&index) else {
            return Err(ActionResult::refused(action, format!("No element #{index} in the last capture. Capture again and use an index from it."), Some("stale")));
        };
        match (&e.token, e.center()) {
            (Some(tok), _) => Ok(obj(json!({"pid": t.pid, "element_token": tok}))),
            (None, Some((x, y))) => Ok(obj(json!({"pid": t.pid, "window_id": t.window_id, "x": x, "y": y}))),
            (None, None) => Err(ActionResult::refused(action, format!("Element #{index} has no handle or position; capture again."), Some("stale"))),
        }
    }

    /// Make sure the driver holds a screenshot frame of the target before a
    /// pixel action: a screenshot-only read (the tree's tokens stay valid).
    fn arm_frame(&mut self, action: &str) -> Result<(), ActionResult> {
        if self.framed {
            return Ok(());
        }
        let t = self.need_target(action)?;
        let args = obj(json!({"pid": t.pid, "window_id": t.window_id, "include_screenshot": true,
            "include_accessibility_tree": false, "max_image_dimension": MAX_IMAGE_DIMENSION}));
        match self.call("get_window_state", args) {
            Ok(env) if !env.is_error && env.image().is_some() => {
                self.framed = true;
                Ok(())
            }
            Ok(env) => Err(ActionResult::refused(action, format!("could not take the screenshot pixel coordinates refer to: {}", env.message()), Some("screenshot_unavailable"))),
            Err(e) => Err(ActionResult::refused(action, format!("cua-driver error: {e}"), Some("transport"))),
        }
    }

    fn xy(&self, action: &str, x: i64, y: i64) -> Result<Map<String, Value>, ActionResult> {
        let t = self.need_target(action)?;
        Ok(obj(json!({"pid": t.pid, "window_id": t.window_id, "x": x, "y": y})))
    }

    fn with_delivery(&self, tool: &str, args: &mut Map<String, Value>, delivery: Option<&str>) -> Result<(), ActionResult> {
        match delivery {
            None | Some("background") => Ok(()),
            Some("foreground") if self.schemas.accepts(tool, "delivery_mode") => {
                args.insert("delivery_mode".into(), json!("foreground"));
                Ok(())
            }
            Some("foreground") => Err(ActionResult {
                delivery_mode: Some("foreground".into()),
                ..ActionResult::refused(tool, "This driver's action schema has no delivery_mode, so foreground delivery is unavailable. Use another verified rung.", Some("foreground_unsupported"))
            }),
            Some(other) => Err(ActionResult::refused(tool, format!("unknown delivery_mode {other:?}: use background or foreground"), Some("bad_delivery_mode"))),
        }
    }

    /// Run one driver action and read its verdict.
    pub fn action(&mut self, tool: &str, args: Map<String, Value>, delivery: Option<&str>) -> ActionResult {
        match self.call(tool, args) {
            Ok(env) => parse::action_result(tool, &env, delivery),
            Err(e) => ActionResult::refused(tool, format!("cua-driver error: {e}"), Some("transport")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn click(&mut self, element: Option<u32>, xy: Option<(i64, i64)>, button: &str, count: u32, modifiers: &[String], delivery: Option<&str>) -> ActionResult {
        let button = button.to_lowercase();
        if !["left", "right", "middle"].contains(&button.as_str()) {
            return ActionResult::refused("click", format!("unknown button {button:?}: left, right or middle"), Some("bad_button"));
        }
        let addressed = match (element, xy) {
            (Some(i), _) => self.address("click", i),
            (None, Some((x, y))) => self.xy("click", x, y),
            (None, None) => Err(ActionResult::refused("click", "click needs element= or coordinate=[x, y]", Some("bad_arguments"))),
        };
        let mut args = match addressed {
            Ok(a) => a,
            Err(r) => return r,
        };
        if args.contains_key("x")
            && let Err(r) = self.arm_frame("click")
        {
            return r;
        }
        // Builds whose click has no `count` keep a separate double_click tool.
        let tool = if count == 2 && !self.schemas.accepts("click", "count") && self.schemas.has_tool("double_click") {
            "double_click"
        } else {
            "click"
        };
        if count > 1 && !self.schemas.accepts(tool, "count") && tool == "click" {
            return ActionResult::refused("click", "This driver build cannot multi-click.", Some("unsupported_action"));
        }
        args.insert("button".into(), json!(button));
        if count > 1 && tool == "click" {
            args.insert("count".into(), json!(count.min(3)));
        }
        if !modifiers.is_empty() {
            args.insert("modifier".into(), json!(modifiers));
        }
        if let Err(r) = self.with_delivery(tool, &mut args, delivery) {
            return r;
        }
        let mut r = self.action(tool, args, delivery);
        r.action = "click".into();
        r
    }

    pub fn drag(&mut self, from: (i64, i64), to: (i64, i64), button: &str, modifiers: &[String], delivery: Option<&str>) -> ActionResult {
        let t = match self.need_target("drag") {
            Ok(t) => t,
            Err(r) => return r,
        };
        if let Err(r) = self.arm_frame("drag") {
            return r;
        }
        let mut args = obj(json!({"pid": t.pid, "window_id": t.window_id, "from_x": from.0, "from_y": from.1, "to_x": to.0, "to_y": to.1, "button": button}));
        if !modifiers.is_empty() {
            args.insert("modifier".into(), json!(modifiers));
        }
        if let Err(r) = self.with_delivery("drag", &mut args, delivery) {
            return r;
        }
        self.action("drag", args, delivery)
    }

    /// An element's center, for drags (which take coordinates only).
    pub fn center_of(&self, index: u32) -> Option<(i64, i64)> {
        self.elements.get(&index).and_then(UiElement::center)
    }

    pub fn scroll(&mut self, direction: &str, amount: u32, element: Option<u32>, xy: Option<(i64, i64)>, delivery: Option<&str>) -> ActionResult {
        if !["up", "down", "left", "right"].contains(&direction) {
            return ActionResult::refused("scroll", format!("unknown direction {direction:?}"), Some("bad_arguments"));
        }
        let t = match self.need_target("scroll") {
            Ok(t) => t,
            Err(r) => return r,
        };
        let mut args = match (element, xy) {
            (Some(i), _) => match self.address("scroll", i) {
                Ok(a) => a,
                Err(r) => return r,
            },
            (None, Some((x, y))) => obj(json!({"pid": t.pid, "window_id": t.window_id, "x": x, "y": y})),
            (None, None) => obj(json!({"pid": t.pid, "window_id": t.window_id})),
        };
        if args.contains_key("x")
            && let Err(r) = self.arm_frame("scroll")
        {
            return r;
        }
        args.insert("direction".into(), json!(direction));
        args.insert("amount".into(), json!(amount.clamp(1, 50)));
        if let Err(r) = self.with_delivery("scroll", &mut args, delivery) {
            return r;
        }
        self.action("scroll", args, delivery)
    }

    pub fn type_text(&mut self, text: &str, element: Option<u32>, delivery: Option<&str>) -> ActionResult {
        let t = match self.need_target("type_text") {
            Ok(t) => t,
            Err(r) => return r,
        };
        let mut args = match element {
            Some(i) => match self.address("type_text", i) {
                Ok(a) => a,
                Err(r) => return r,
            },
            None => obj(json!({"pid": t.pid, "window_id": t.window_id})),
        };
        if args.contains_key("x")
            && let Err(r) = self.arm_frame("type_text")
        {
            return r;
        }
        args.insert("text".into(), json!(text));
        if let Err(r) = self.with_delivery("type_text", &mut args, delivery) {
            return r;
        }
        let mut r = self.action("type_text", args, delivery);
        r.action = "type".into();
        r
    }

    pub fn key(&mut self, keys: &str, delivery: Option<&str>) -> ActionResult {
        let t = match self.need_target("key") {
            Ok(t) => t,
            Err(r) => return r,
        };
        let (key, mods) = parse::key_combo(keys);
        let Some(key) = key else {
            return ActionResult::refused("key", format!("Could not read a key from {keys:?}"), Some("bad_arguments"));
        };
        let (tool, mut args) = if mods.is_empty() {
            ("press_key", obj(json!({"pid": t.pid, "window_id": t.window_id, "key": key})))
        } else {
            let mut all = mods.clone();
            all.push(key);
            ("hotkey", obj(json!({"pid": t.pid, "window_id": t.window_id, "keys": all})))
        };
        if let Err(r) = self.with_delivery(tool, &mut args, delivery) {
            return r;
        }
        let mut r = self.action(tool, args, delivery);
        r.action = "key".into();
        r
    }

    pub fn set_value(&mut self, element: u32, value: &str) -> ActionResult {
        let mut args = match self.address("set_value", element) {
            Ok(a) if a.contains_key("element_token") => a,
            Ok(_) => return ActionResult::refused("set_value", format!("#{element} is OCR text, not a control; click it and type instead"), Some("bad_arguments")),
            Err(r) => return r,
        };
        args.insert("value".into(), json!(value));
        self.action("set_value", args, None)
    }

    /// `run_steps`: one round trip for a whole sub-flow.
    pub fn run_steps(&mut self, steps: Vec<Value>, delivery: Option<&str>, observe: bool) -> Result<Envelope, String> {
        let mut args = obj(json!({"steps": steps}));
        if let Some(t) = &self.target {
            args.insert("pid".into(), json!(t.pid));
            args.insert("window_id".into(), json!(t.window_id));
        }
        if let Some(d) = delivery.filter(|d| *d == "foreground") {
            args.insert("delivery_mode".into(), json!(d));
        }
        if observe {
            // A full read (`since: null`), so the rows after the batch are
            // complete and their indices fresh.
            args.insert(
                "observe".into(),
                json!({"tree_format": "elements", "max_elements": AX_MAX_ELEMENTS, "since": null, "include_screenshot": false}),
            );
        }
        let env = self.call("run_steps", args)?;
        // The batch acted: the snapshot behind our indices is old now. The
        // end observation (`observation.state`) brings fresh ones.
        let state = env.structured.get("observation").and_then(|o| o.get("state")).and_then(Value::as_object).cloned();
        match state {
            Some(o) if o.get("elements").is_some() => {
                let inner = Envelope { structured: o.clone(), ..Envelope::default() };
                let sid = o.get("snapshot_id").and_then(Value::as_str).map(str::to_string);
                self.replace_rows(&inner, sid.as_deref());
            }
            _ if observe => {
                // Stale indices must not survive an unobserved batch.
                self.elements.clear();
                self.raw_rows.clear();
                self.snapshot = None;
            }
            _ => {}
        }
        Ok(env)
    }

    /// Checks that must hold now, as `run_steps` check-only steps (four
    /// per step): `{role?, name?, text?, gone?, value?, value_contains?,
    /// enabled?, selected?}`, each polled until it holds or times out.
    pub fn verify(&mut self, expect: Vec<Value>) -> Result<Envelope, String> {
        if self.target.is_none() {
            return Err("No target window: capture first.".into());
        }
        let steps: Vec<Value> = expect.chunks(4).map(|c| json!({"expect": c})).collect();
        self.run_steps(steps, None, false)
    }
}

/// The window frame's own controls: title bar, system menu and caption buttons.
pub fn is_chrome(e: &UiElement) -> bool {
    let role = e.role.to_lowercase();
    if role == "titlebar" || role == "axtitlebar" || role.starts_with("axclosebutton") || role.starts_with("axminimizebutton") || role.starts_with("axzoombutton") {
        return true;
    }
    let label = e.label.trim();
    matches!(role.as_str(), "button" | "menuitem" | "push button" | "menu item")
        && ["System", "Minimize", "Maximize", "Restore", "Close"]
            .iter()
            .any(|c| label == *c || label.starts_with(&format!("{c} ")))
        && !label.eq_ignore_ascii_case("close tab")
}

/// An app name as people say it: lowercase, without `.exe` or `.app`.
fn norm_app(name: &str) -> String {
    let n = name.trim().to_lowercase();
    n.strip_suffix(".exe").or_else(|| n.strip_suffix(".app")).unwrap_or(&n).trim().to_string()
}

/// Processes that only host other apps' windows: the window title names
/// the app (Calculator, Settings and other UWP apps on Windows).
fn is_host(app: &str) -> bool {
    matches!(norm_app(app).as_str(), "applicationframehost")
}

fn named(w: &Window, names: &[&str]) -> bool {
    let hay = format!("{} {}", w.app, w.title).to_lowercase();
    names.iter().any(|n| hay.contains(n))
}

/// `_NET_ACTIVE_WINDOW` via xprop (X11 only; never fails loudly).
fn linux_active_window() -> Option<u64> {
    if !cfg!(target_os = "linux") || std::env::var_os("DISPLAY").is_none() {
        return None;
    }
    let out = std::process::Command::new("xprop").args(["-root", "_NET_ACTIVE_WINDOW"]).stdin(std::process::Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let hex = text.split_whitespace().find(|t| t.starts_with("0x"))?;
    u64::from_str_radix(hex.trim_start_matches("0x").trim_end_matches(','), 16).ok().filter(|w| *w > 0)
}

/// Why discovery found no windows: the usual causes per platform.
fn empty_discovery_reason() -> String {
    if cfg!(target_os = "linux") && std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return "window discovery found nothing: no DISPLAY or WAYLAND_DISPLAY is set".into();
    }
    if cfg!(target_os = "macos") {
        return "window discovery found nothing: macOS reports no shareable display (asleep or headless), or Screen Recording is off for CuaDriver".into();
    }
    "window discovery found nothing: the desktop may be locked, or the driver lacks access; run the checks in Dove → Settings → Plugins → Computer use".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeDriver;

    fn cua(fake: FakeDriver) -> Cua<FakeDriver> {
        let mut c = Cua::new(fake, "dove-test".into());
        c.start().unwrap();
        c
    }

    #[test]
    fn capture_reads_elements_and_mints_tokens() {
        let mut c = cua(FakeDriver::notepad());
        let cap = c.capture(&CaptureRequest { mode: "auto".into(), app: Some("Notepad".into()), ..Default::default() });
        assert_eq!(cap.app, "Notepad");
        assert_eq!(cap.elements.len(), 4);
        assert!(cap.image.is_none(), "tree-first: no pixels when the tree is rich");
        let gws = c.driver.calls_to("get_window_state").pop().unwrap();
        assert_eq!(gws["include_screenshot"], false);
        assert_eq!(gws["tree_format"], "elements");
        assert!(gws.get("session").is_some());
        // Element clicks send the token, never an index the schema refuses.
        let r = c.click(Some(2), None, "left", 1, &[], None);
        assert!(r.ok, "{r:?}");
        let click = c.driver.calls_to("click").pop().unwrap();
        assert_eq!(click["element_token"], "s0000000a:2");
        assert!(click.get("element_index").is_none());
    }

    #[test]
    fn minimized_windows_act_by_index_only() {
        let mut c = cua(FakeDriver::minimized());
        let cap = c.capture(&CaptureRequest { mode: "auto".into(), app: Some("Notepad".into()), ..Default::default() });
        assert!(cap.note.contains("minimized"), "{}", cap.note);
        assert!(cap.image.is_none(), "no screenshot of a minimized window");
        assert!(cap.elements.iter().all(|e| !e.bounds_known()), "no coordinates to click by");
        assert!(c.click(Some(1), None, "left", 1, &[], None).ok, "rows still act by token");
        assert_eq!(c.driver.calls_to("click").pop().unwrap()["element_token"], "s0000000a:1");
    }

    #[test]
    fn minimized_windows_are_still_found_by_name() {
        let mut c = cua(FakeDriver::notepad());
        let cap = c.capture(&CaptureRequest { mode: "ax".into(), app: Some("Mail".into()), ..Default::default() });
        assert!(cap.note.is_empty(), "{}", cap.note);
        assert_eq!(c.target.as_ref().map(|t| t.app.as_str()), Some("Mail.exe"));
        assert_eq!(c.target.as_ref().map(|t| t.window_id), Some(301));
        let none = c.capture(&CaptureRequest { mode: "ax".into(), app: Some("Nothing".into()), ..Default::default() });
        assert!(none.note.starts_with("no window matched"), "{}", none.note);
    }

    #[test]
    fn exact_rereads_skip_discovery() {
        let mut c = cua(FakeDriver::notepad());
        c.capture(&CaptureRequest { mode: "ax".into(), pid: Some(100), window_id: Some(200), ..Default::default() });
        assert_eq!(c.driver.calls_to("list_windows").len(), 1, "a new exact target is named once");
        let cap = c.capture(&CaptureRequest { mode: "ax".into(), pid: Some(100), window_id: Some(200), ..Default::default() });
        assert_eq!(c.driver.calls_to("list_windows").len(), 1, "a re-read of the target does not list windows again");
        assert_eq!(cap.app, "Notepad");
    }

    #[test]
    fn app_matching_prefers_exact_names() {
        let mut c = cua(FakeDriver::notepad());
        let ws = vec![
            Window { app: "Visual Studio Code".into(), pid: 1, window_id: 1, title: String::new(), off_screen: false, z: 9.0 },
            Window { app: "Code".into(), pid: 2, window_id: 2, title: String::new(), off_screen: false, z: 1.0 },
        ];
        assert_eq!(c.match_app(&ws, "code")[0].pid, 2);
        assert_eq!(c.match_app(&ws, "studio")[0].pid, 1);
        assert!(c.match_app(&ws, "nothing").is_empty());
        // Names as people say them, and UWP apps by their window title, all
        // without the slow list_apps call.
        let listed = c.driver.calls_to("list_apps").len();
        let ws = vec![
            Window { app: "Notepad.exe".into(), pid: 3, window_id: 3, title: "notes.txt - Notepad".into(), off_screen: false, z: 3.0 },
            Window { app: "ApplicationFrameHost.exe".into(), pid: 4, window_id: 4, title: "Settings".into(), off_screen: false, z: 2.0 },
            Window { app: "ApplicationFrameHost.exe".into(), pid: 4, window_id: 5, title: "Calculator".into(), off_screen: false, z: 1.0 },
        ];
        assert_eq!(c.match_app(&ws, "Notepad")[0].pid, 3);
        assert_eq!(c.match_app(&ws, "calculator")[0].window_id, 5);
        assert_eq!(c.driver.calls_to("list_apps").len(), listed, "no list_apps for names the windows already answer");
        assert!(c.match_app(&ws, "applicationframehost").len() == 2, "the host itself still matches by name");
    }

    #[test]
    fn chrome_only_trees_count_as_sparse() {
        let chrome = |i: u32, role: &str, label: &str| UiElement {
            index: i, role: role.into(), label: label.into(), bounds: (1, 1, 10, 10), enabled: true,
            token: Some(format!("s1:{i}")), source: "tree", ..UiElement::default()
        };
        assert!(is_chrome(&chrome(0, "TitleBar", "notes.txt - Notepad")));
        assert!(is_chrome(&chrome(1, "MenuItem", "System")));
        assert!(is_chrome(&chrome(2, "Button", "Close Calculator")));
        assert!(!is_chrome(&chrome(3, "Button", "Close Tab")));
        assert!(!is_chrome(&chrome(4, "Button", "Save")));
        assert!(!is_chrome(&chrome(5, "Button", "Closed captions")));
    }

    #[test]
    fn sparse_trees_get_a_screenshot_in_auto_mode() {
        let mut c = cua(FakeDriver::canvas());
        let cap = c.capture(&CaptureRequest { mode: "auto".into(), app: Some("Paint".into()), ..Default::default() });
        assert!(cap.image.is_some());
        assert!(cap.image_reason.is_some());
    }

    #[test]
    fn pixel_actions_rearm_the_screenshot_frame() {
        let mut c = cua(FakeDriver::notepad());
        // A text-only read: the driver holds no frame for pixels.
        c.capture(&CaptureRequest { mode: "ax".into(), app: Some("Notepad".into()), ..Default::default() });
        let reads = c.driver.calls_to("get_window_state").len();
        assert!(c.click(None, Some((40, 50)), "left", 1, &[], None).ok);
        let calls = c.driver.calls_to("get_window_state");
        assert_eq!(calls.len(), reads + 1, "one screenshot-only read before the pixel click");
        assert_eq!(calls.last().unwrap()["include_accessibility_tree"], false);
        assert_eq!(calls.last().unwrap()["include_screenshot"], true);
        // Armed now: the next pixel click needs no read; element clicks never do.
        c.click(None, Some((41, 51)), "left", 1, &[], None);
        c.click(Some(2), None, "left", 1, &[], None);
        assert_eq!(c.driver.calls_to("get_window_state").len(), reads + 1);
        // A screenshot capture arms it too.
        c.capture(&CaptureRequest { mode: "som".into(), app: Some("Notepad".into()), ..Default::default() });
        let n = c.driver.calls_to("get_window_state").len();
        c.drag((1, 1), (5, 5), "left", &[], None);
        assert_eq!(c.driver.calls_to("get_window_state").len(), n, "a som capture already armed the frame");
    }

    #[test]
    fn foreground_needs_the_schema() {
        let mut c = cua(FakeDriver::notepad());
        c.capture(&CaptureRequest { mode: "ax".into(), app: Some("Notepad".into()), ..Default::default() });
        let r = c.type_text("hi", None, Some("foreground"));
        assert!(r.ok);
        assert_eq!(c.driver.calls_to("type_text").pop().unwrap()["delivery_mode"], "foreground");
        assert_eq!(c.click(Some(99), None, "left", 1, &[], None).code.as_deref(), Some("stale"));
        assert_eq!(c.key("ctrl+s", None).action, "key");
        assert_eq!(c.driver.calls_to("hotkey").pop().unwrap()["keys"], json!(["ctrl", "s"]));
        c.key("return", None);
        assert_eq!(c.driver.calls_to("press_key").pop().unwrap()["key"], "return");
    }

    #[test]
    fn since_diffs_merge_and_remint_tokens() {
        let mut c = cua(FakeDriver::notepad());
        c.capture(&CaptureRequest { mode: "ax".into(), app: Some("Notepad".into()), ..Default::default() });
        // No change: the same rows, tokens re-minted on the new snapshot.
        let cap = c.capture(&CaptureRequest { mode: "ax".into(), app: Some("Notepad".into()), since: Some("latest".into()), ..Default::default() });
        assert!(cap.diff.as_deref().is_some_and(|d| d.starts_with("no change")));
        assert_eq!(cap.elements.len(), 4);
        assert_eq!(c.element(1).unwrap().token.as_deref(), Some("s0000000b:1"));
        let since_read = c.driver.calls_to("get_window_state").pop().unwrap();
        assert_eq!(since_read["tree_format"], "markdown", "0.38 refuses elements-format since reads");
        // A change: the diff for the model, then a full read for exact rows.
        c.driver.next_diff = Some(("~ [2] Button \"Saved\"
- [3] Edit".into(), "s0000000c".into()));
        let cap = c.capture(&CaptureRequest { mode: "ax".into(), app: Some("Notepad".into()), since: Some("latest".into()), ..Default::default() });
        assert!(cap.diff.as_deref().is_some_and(|d| d.contains("Saved")));
        assert_eq!(cap.elements.len(), 3, "row 3 was removed");
        assert_eq!(c.element(2).unwrap().label, "Saved");
        assert_eq!(c.driver.calls_to("get_window_state").pop().unwrap()["tree_format"], "elements");
    }
}
