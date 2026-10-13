//! Device-local browser tabs. GPUI owns chrome; the native host owns pages.
//!
//! Ported from Zeron (MIT): WebView2 in visual-hosting mode on Windows
//! (composited through zui's overlay plane), WKWebView on macOS, and an
//! offscreen WebKitGTK helper process on Linux.
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
use linux as native;
#[cfg(target_os = "macos")]
use macos as native;
#[cfg(windows)]
mod windows_webview;
#[cfg(windows)]
use windows_webview as native;
mod address;
pub mod agent;
pub mod control;
pub mod model;
mod view;

use address::{AddressEvent, AddressInput};
use gpui::{App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, Subscription, Window};
use model::{PageState, Presentation};
gpui::actions!(browser, [Reload, FocusAddress, NewTab, CloseTab, Back, Forward]);

pub(crate) fn bind_keys(cx: &mut App) {
    let primary = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
    cx.bind_keys([
        gpui::KeyBinding::new(&format!("{primary}-l"), FocusAddress, Some("Browser")),
        gpui::KeyBinding::new(&format!("{primary}-t"), NewTab, Some("Browser")),
        gpui::KeyBinding::new(&format!("{primary}-w"), CloseTab, Some("Browser")),
        gpui::KeyBinding::new(&format!("{primary}-["), Back, Some("Browser")),
        gpui::KeyBinding::new(&format!("{primary}-]"), Forward, Some("Browser")),
        gpui::KeyBinding::new(&format!("{primary}-r"), Reload, Some("Browser")),
        gpui::KeyBinding::new("f5", Reload, Some("Browser")),
    ]);
}

/// The tokio runtime for favicon downloads (the engine's).
static RUNTIME: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();

pub fn set_runtime(handle: tokio::runtime::Handle) {
    let _ = RUNTIME.set(handle);
}

fn runtime() -> tokio::runtime::Handle {
    RUNTIME.get().cloned().unwrap_or_else(|| tokio::runtime::Handle::current())
}

/// Where a page script's (or DevTools call's) result goes. Called once, on
/// whichever thread the native host answers on.
pub type EvalDone = Box<dyn FnOnce(Result<serde_json::Value, String>) + Send + 'static>;

#[derive(Clone, Debug)]
pub enum BrowserEvent {
    Changed,
    NewTab(Option<String>),
    Close,
}

/// One human-like action a harness can take on the in-app page.
#[derive(Clone, Debug)]
pub enum Drive {
    Navigate(String),
    Click { selector: String },
    Type { selector: String, text: String },
    Press { key: String },
    Scroll { dy: f32, selector: Option<String> },
    Snapshot,
}

impl Drive {
    /// Map a tool name + JSON input onto a page action. Unknown tools return None.
    pub fn from_tool(name: &str, input: &serde_json::Value) -> Option<Self> {
        let n = name.rsplit(['/', ':', '.']).next().unwrap_or(name).to_ascii_lowercase();
        let s = |k: &str| {
            input
                .get(k)
                .and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_i64().map(|n| n.to_string())))
                .unwrap_or_default()
        };
        let selector = {
            let v = s("selector");
            if !v.is_empty() {
                v
            } else {
                let v = s("ref");
                if !v.is_empty() { v } else { s("target") }
            }
        };
        match n.as_str() {
            "browser_navigate" | "navigate" | "open_browser" | "browser_open" | "open" if input.get("url").is_some() || input.get("href").is_some() => {
                let url = s("url");
                let url = if url.is_empty() { s("href") } else { url };
                (!url.is_empty()).then_some(Self::Navigate(url))
            }
            "browser" => {
                let url = s("url");
                if !url.is_empty() {
                    Some(Self::Navigate(url))
                } else {
                    None
                }
            }
            "browser_click" | "click" | "browser_tap" => (!selector.is_empty()).then(|| Self::Click { selector }),
            "browser_type" | "type" | "fill" | "browser_fill" => {
                let text = s("text");
                let text = if text.is_empty() { s("value") } else { text };
                (!selector.is_empty()).then(|| Self::Type { selector, text })
            }
            "browser_press" | "press" | "browser_key" => {
                let key = s("key");
                let key = if key.is_empty() { s("keys") } else { key };
                (!key.is_empty()).then(|| Self::Press { key })
            }
            "browser_scroll" | "scroll" => {
                let dy = input.get("dy").or_else(|| input.get("delta")).and_then(|v| v.as_f64()).unwrap_or(400.0) as f32;
                Some(Self::Scroll { dy, selector: (!selector.is_empty()).then_some(selector) })
            }
            "browser_snapshot" | "snapshot" | "browser_screenshot" => Some(Self::Snapshot),
            _ => None,
        }
    }
}

const PAGE_API: &str = r#"
(() => {
  if (window.__doveDrive) return;
  function find(sel) {
    if (!sel) return null;
    try { const n = document.querySelector(sel); if (n) return n; } catch (e) {}
    const want = String(sel).trim().toLowerCase();
    const nodes = document.querySelectorAll('a,button,input,textarea,select,[role="button"],[role="link"],label,summary,[contenteditable]');
    for (const el of nodes) {
      const label = (el.innerText || el.value || el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.id || el.name || '').trim().toLowerCase();
      if (label && (label === want || label.includes(want))) return el;
    }
    return null;
  }
  window.__doveDrive = {
    click(sel) { const el = find(sel); if (!el) return; el.scrollIntoView({block:'center', inline:'nearest'}); el.click(); },
    type(sel, text) {
      const el = find(sel) || document.activeElement;
      if (!el) return;
      el.focus();
      if ('value' in el) {
        el.value = text;
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
      } else {
        el.textContent = text;
      }
    },
    press(key) {
      const el = document.activeElement || document.body;
      const ev = { key, code: key, bubbles: true, cancelable: true };
      el.dispatchEvent(new KeyboardEvent('keydown', ev));
      el.dispatchEvent(new KeyboardEvent('keyup', ev));
      if (key === 'Enter' && el.form) el.form.requestSubmit?.();
    },
    scroll(dy, sel) {
      const n = sel ? find(sel) : null;
      if (n) n.scrollBy(0, dy); else window.scrollBy(0, dy);
    },
    snapshot() {
      const els = [...document.querySelectorAll('a,button,input,textarea,select,h1,h2,h3,[role]')].slice(0, 120);
      return JSON.stringify({
        title: document.title,
        url: location.href,
        nodes: els.map((el, i) => ({
          i,
          tag: el.tagName.toLowerCase(),
          text: (el.innerText || el.value || '').trim().slice(0, 80),
          name: el.getAttribute('aria-label') || el.name || el.id || null,
        })),
      });
    },
  };
})()
"#;

/// A window/profile's ephemeral website data, allocated on first navigation.
#[derive(Clone, Default)]
pub struct BrowserContext {
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    data: native::BrowserData,
}

pub struct BrowserSurface {
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    context: BrowserContext,
    address: Entity<AddressInput>,
    focus: FocusHandle,
    pub page: PageState,
    pub favicon: Option<std::sync::Arc<gpui::Image>>,
    address_edited: bool,
    validation: Option<String>,
    remote: bool,
    previews: model::PreviewSnapshot,
    /// Dev-server discovery is on (the thread's shells feed it).
    previews_enabled: bool,
    previews_loading: bool,
    previews_task: Option<gpui::Task<()>>,
    #[cfg(feature = "browser-fixture")]
    fixture_preview_open: std::rc::Rc<std::cell::Cell<Option<gpui::Point<gpui::Pixels>>>>,
    presentation: Presentation,
    #[cfg(target_os = "macos")]
    resize_inset: gpui::Pixels,
    #[cfg(any(target_os = "macos", windows))]
    right_occlusion: gpui::Pixels,
    _input_sub: Subscription,
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    native: Option<native::NativePage>,
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    native_tx: tokio::sync::mpsc::Sender<native::NativeEvent>,
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    _native_task: gpui::Task<()>,
    #[cfg(any(target_os = "macos", windows))]
    favicon_task: Option<gpui::Task<()>>,
    #[cfg(any(target_os = "macos", windows))]
    favicon_generation: u64,
    /// The page's pointer cursor (Windows: GPUI owns the cursor).
    #[cfg(windows)]
    page_cursor: gpui::CursorStyle,
    /// What an agent is doing on this page right now.
    pub agent_activity: Option<gpui::SharedString>,
    agent_generation: u64,
}

impl EventEmitter<BrowserEvent> for BrowserSurface {}
impl Focusable for BrowserSurface {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BrowserSurface {
    pub fn new(context: BrowserContext, remote: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let address = cx.new(|cx| AddressInput::new("Website or localhost:3000", window, cx));
        let input_sub = cx.subscribe(&address, |this, _, event, cx| {
            if matches!(event, AddressEvent::Edited) {
                this.address_edited = this.address.read(cx).text(cx) != this.page.url.as_deref().unwrap_or_default();
                this.validation = None;
                cx.notify();
            }
        });
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        let (native_tx, mut events) = tokio::sync::mpsc::channel(64);
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        let native_task = cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this.update_in(cx, |this, window, cx| this.on_native_event(event, window, cx)).is_err() {
                    break;
                }
            }
        });
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        let _ = (window, context);
        Self {
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            context,
            address,
            focus: cx.focus_handle(),
            page: PageState::default(),
            favicon: None,
            address_edited: false,
            validation: None,
            remote,
            previews: model::PreviewSnapshot::default(),
            previews_enabled: false,
            previews_loading: true,
            previews_task: None,
            #[cfg(feature = "browser-fixture")]
            fixture_preview_open: Default::default(),
            presentation: Presentation::Hidden,
            #[cfg(target_os = "macos")]
            resize_inset: gpui::px(0.0),
            #[cfg(any(target_os = "macos", windows))]
            right_occlusion: gpui::px(0.0),
            _input_sub: input_sub,
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            native: None,
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            native_tx,
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            _native_task: native_task,
            #[cfg(any(target_os = "macos", windows))]
            favicon_task: None,
            #[cfg(any(target_os = "macos", windows))]
            favicon_generation: 0,
            #[cfg(windows)]
            page_cursor: gpui::CursorStyle::Arrow,
            agent_activity: None,
            agent_generation: 0,
        }
    }

    /// Show (or, after a short linger, hide) the agent activity pill.
    pub fn set_agent_activity(&mut self, text: Option<String>, cx: &mut Context<Self>) {
        self.agent_generation += 1;
        match text {
            Some(t) => {
                self.agent_activity = Some(t.into());
                cx.notify();
            }
            None => {
                let generation = self.agent_generation;
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(1800)).await;
                    let _ = this.update(cx, |b, cx| {
                        if b.agent_generation == generation {
                            b.agent_activity = None;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
        }
    }

    pub fn title(&self) -> gpui::SharedString {
        self.page.label().into()
    }
    pub fn set_remote(&mut self, remote: bool) {
        self.remote = remote;
    }
    /// App shortcuts the page must hand back to Dove instead of consuming.
    pub fn set_shortcuts(&mut self, combos: Vec<String>) {
        #[cfg(any(target_os = "macos", windows))]
        if let Some(native) = &self.native {
            native.set_shortcuts(combos);
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        let _ = combos;
    }

    pub fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", windows))]
        if let Some(native) = &self.native {
            native.focus_chrome();
        }
        window.focus(&self.address.focus_handle(cx), cx);
        self.address.update(cx, |a, cx| a.select_all(window, cx));
    }

    /// Reserve the overlapping part of the shell divider for GPUI hit testing.
    #[cfg(target_os = "macos")]
    pub fn set_resize_inset(&mut self, inset: gpui::Pixels, cx: &mut Context<Self>) {
        if self.resize_inset != inset {
            self.resize_inset = inset;
            cx.notify();
        }
    }

    /// Crop a GPUI overlay out of both native painting and native hit testing.
    #[cfg(any(target_os = "macos", windows))]
    pub fn set_right_occlusion(&mut self, width: gpui::Pixels, cx: &mut Context<Self>) {
        if self.right_occlusion != width {
            self.right_occlusion = width;
            cx.notify();
        }
    }

    pub fn set_presentation(&mut self, presentation: Presentation, cx: &mut Context<Self>) {
        if self.presentation == presentation {
            return;
        }
        self.presentation = presentation;
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        if let Some(native) = &mut self.native {
            native.present(presentation);
        }
        cx.notify();
    }

    #[cfg(feature = "browser-fixture")]
    pub fn fixture_previews(&self) -> zeron_proto::PreviewSnapshot {
        self.previews.clone()
    }
    #[cfg(feature = "browser-fixture")]
    pub fn fixture_preview_open_position(&self) -> Option<gpui::Point<gpui::Pixels>> {
        self.fixture_preview_open.get()
    }

    /// Dev servers the thread's shells printed, offered on the empty tab.
    pub fn set_previews(&mut self, urls: Vec<String>, cx: &mut Context<Self>) {
        let snapshot = model::PreviewSnapshot::from_urls(&urls);
        if !self.previews_enabled || snapshot != self.previews {
            self.previews_enabled = true;
            self.previews = snapshot;
            self.previews_loading = false;
            cx.notify();
        }
    }

    pub fn navigate(&mut self, input: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_with(input, true, window, cx);
    }

    /// Navigate without taking keyboard focus (an agent driving the page
    /// must not pull focus from the composer).
    pub fn navigate_quiet(&mut self, input: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_with(input, false, window, cx);
    }

    /// The agent is driving this tab. The page stays on screen; focus stays
    /// with the composer.
    pub fn set_headless(&mut self, _on: bool) {}

    fn navigate_with(&mut self, input: &str, focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        let url = match model::normalize_address(input) {
            Ok(url) => url,
            Err(message) => {
                self.validation = Some(message.into());
                cx.notify();
                return;
            }
        };
        self.validation = None;
        self.address.update(cx, |input, cx| input.set_text(url.clone(), cx));
        self.address_edited = false;
        self.page.url = Some(url.clone());
        self.page.title.clear();
        self.page.error = None;
        self.clear_favicon(cx);
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        {
            #[cfg(any(target_os = "macos", windows))]
            {
                self.favicon_generation += 1;
                self.favicon_task = None;
            }
            let result = if let Some(native) = &self.native {
                native.load(&url)
            } else {
                native::NativePage::new(window, &self.context.data, self.native_tx.clone())
                    .map(|mut native| {
                        native.present(self.presentation);
                        self.native = Some(native);
                    })
                    .and_then(|_| self.native.as_ref().unwrap().load(&url))
            };
            self.page.loading = result.is_ok();
            if let Err(error) = result {
                self.page.error = Some(format!("Could not open this page: {error}"));
            }
            if focus {
                window.focus(&self.focus, cx);
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            let _ = (window, focus);
            cx.open_url(&url);
        }
        cx.emit(BrowserEvent::Changed);
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.address.read(cx).text(cx);
        self.navigate(&url, window, cx);
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        if let Some(native) = &self.native {
            if self.page.error.is_some() {
                if let Some(url) = &self.page.url {
                    let _ = native.load(url);
                }
            } else {
                native.reload();
            }
            self.page.error = None;
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        self.open_external(cx);
        cx.notify();
    }

    fn open_external(&self, cx: &mut Context<Self>) {
        if let Some(url) = self.page.url.as_deref().filter(|url| model::allowed_navigation(url)) {
            cx.open_url(url);
        }
    }

    fn history(&mut self, forward: bool) {
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        if let Some(native) = &self.native {
            native.history(forward);
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        let _ = forward;
    }

    /// Explicitly close even if an async callback temporarily retains an entity.
    fn clear_favicon(&mut self, cx: &mut Context<Self>) {
        if let Some(image) = self.favicon.take() {
            cx.defer(move |cx| gpui::ImageSource::Image(image).evict(None, cx));
        }
    }

    /// Run a page action as a person would: open, click, type, press, or scroll.
    pub fn drive(&mut self, action: Drive, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Drive::Navigate(url) => self.navigate(&url, window, cx),
            Drive::Click { selector } => self.eval(&format!("{PAGE_API}; __doveDrive.click({});", json_str(&selector))),
            Drive::Type { selector, text } => {
                self.eval(&format!("{PAGE_API}; __doveDrive.type({}, {});", json_str(&selector), json_str(&text)))
            }
            Drive::Press { key } => self.eval(&format!("{PAGE_API}; __doveDrive.press({});", json_str(&key))),
            Drive::Scroll { dy, selector } => {
                let sel = selector.as_deref().map(json_str).unwrap_or_else(|| "null".into());
                self.eval(&format!("{PAGE_API}; __doveDrive.scroll({dy}, {sel});"))
            }
            Drive::Snapshot => self.eval(&format!("{PAGE_API}; __doveDrive.snapshot();")),
        }
        cx.notify();
    }

    /// Run a script in the page; `done` gets its JSON result.
    pub fn eval_value(&self, script: &str, done: EvalDone) {
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        if let Some(native) = &self.native {
            return native.eval_value(script, done);
        }
        let _ = script;
        done(Err("No page is open in this browser tab yet".into()));
    }

    /// One DevTools Protocol call (Windows; elsewhere answers "unsupported").
    pub fn cdp(&self, method: &str, params: &serde_json::Value, done: EvalDone) {
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        if let Some(native) = &self.native {
            return native.cdp(method, params, done);
        }
        let _ = (method, params);
        done(Err("No page is open in this browser tab yet".into()));
    }

    pub fn go_history(&mut self, forward: bool) {
        self.history(forward);
    }

    pub fn reload_page(&mut self, cx: &mut Context<Self>) {
        self.reload(cx);
    }

    /// Whether a native page exists (something has been opened).
    pub fn has_page(&self) -> bool {
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        return self.native.is_some();
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        false
    }

    pub fn eval(&self, script: &str) {
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        if let Some(native) = &self.native {
            native.eval(script);
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        let _ = script;
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.set_presentation(Presentation::Hidden, cx);
        self.clear_favicon(cx);
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        {
            if let Some(native) = &mut self.native {
                native.present(Presentation::Hidden);
            }
            #[cfg(target_os = "linux")]
            if let Some(image) = self.native.as_mut().and_then(|native| native.image.take()) {
                cx.defer(move |cx| gpui::ImageSource::Render(image).evict(None, cx));
            }
            self.native = None;
            #[cfg(any(target_os = "macos", windows))]
            {
                self.favicon_task = None;
                self.favicon_generation += 1;
            }
        }
    }
}

#[cfg(any(target_os = "macos", windows))]
impl BrowserSurface {
    fn on_native_event(&mut self, event: native::NativeEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(native) = &mut self.native else {
            return;
        };
        match event {
            native::NativeEvent::Changed | native::NativeEvent::Finished => {
                let finished = matches!(event, native::NativeEvent::Finished);
                let mut page = native.state();
                // A failed provisional request has no committed WebKit URL.
                if page.url.is_none() {
                    page.url = self.page.url.clone();
                }
                if page.error.is_some() {
                    page.loading = false;
                }
                if page.url != self.page.url || (!self.page.loading && page.loading) {
                    if let Some(image) = self.favicon.take() {
                        cx.defer(move |cx| gpui::ImageSource::Image(image).evict(None, cx));
                    }
                    self.favicon_generation += 1;
                    self.favicon_task = None;
                }
                if !self.address.focus_handle(cx).is_focused(window) {
                    if let Some(url) = &page.url {
                        if self.address.read(cx).text(cx) != *url {
                            self.address.update(cx, |input, cx| input.set_text(url.clone(), cx));
                        }
                    }
                    self.address_edited = false;
                }
                native.present(self.presentation);
                if finished && let Some(url) = &page.url {
                    native.discover_favicon(url.clone());
                }
                if page != self.page {
                    self.page = page;
                    cx.emit(BrowserEvent::Changed);
                }
                cx.notify();
            }
            native::NativeEvent::NewTab(url) => {
                if self.presentation == Presentation::Live {
                    cx.emit(BrowserEvent::NewTab(Some(url)));
                }
            }
            native::NativeEvent::Key(key) => {
                if self.presentation == Presentation::Live {
                    window.focus(&self.focus, cx);
                    window.defer(cx, move |window, cx| {
                        window.dispatch_keystroke(key, cx);
                    });
                }
            }
            #[cfg(windows)]
            native::NativeEvent::Cursor(style) => {
                if self.page_cursor != style {
                    self.page_cursor = style;
                    cx.notify();
                }
            }
            native::NativeEvent::Favicon { page, url } => {
                if self.page.url.as_deref() != Some(&page) || !model::allowed_navigation(&url) {
                    return;
                }
                let generation = self.favicon_generation;
                // reqwest's system proxy has no loopback exemption; a local dev
                // server's favicon must be fetched directly.
                let direct = url::Url::parse(&url).is_ok_and(|parsed| model::loopback(&parsed));
                let download = runtime().spawn(async move {
                    let mut builder = reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(5))
                        .redirect(reqwest::redirect::Policy::limited(3));
                    if direct {
                        builder = builder.no_proxy();
                    }
                    let client = builder.build().ok()?;
                    let mut response = client.get(url).send().await.ok()?.error_for_status().ok()?;
                    if response.content_length().is_some_and(|n| n > 1024 * 1024) {
                        return None;
                    }
                    let mut bytes = Vec::new();
                    while let Some(chunk) = response.chunk().await.ok()? {
                        if bytes.len() + chunk.len() > 1024 * 1024 {
                            return None;
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
                    let mut limits = image::Limits::default();
                    limits.max_image_width = Some(1024);
                    limits.max_image_height = Some(1024);
                    limits.max_alloc = Some(8 * 1024 * 1024);
                    reader.limits(limits);
                    let icon = reader.decode().ok()?.thumbnail(32, 32);
                    let mut png = std::io::Cursor::new(Vec::new());
                    icon.write_to(&mut png, image::ImageFormat::Png).ok()?;
                    Some(png.into_inner())
                });
                self.favicon_task = Some(cx.spawn(async move |this, cx| {
                    let Ok(Some(bytes)) = download.await else {
                        return;
                    };
                    let _ = this.update(cx, |this, cx| {
                        if this.favicon_generation == generation && this.page.url.as_deref() == Some(&page) {
                            this.favicon =
                                Some(std::sync::Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes)));
                            cx.emit(BrowserEvent::Changed);
                            cx.notify();
                        }
                    });
                }));
            }
        }
    }
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}
