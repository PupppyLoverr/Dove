//! The agent · model · effort picker, laid out like Zeron's compact picker.
//!
//! One small card above the composer chip, with three pages:
//! - **Panel**: the effort as the title over the model's name (click it for
//!   the model list), a fast-mode button, an effort slider with one stop per
//!   level, then the model's other traits as rows.
//! - **Models**: back, a strip of provider tabs (Starred first), search, and
//!   every offered model: starred first, then one run per agent.
//! - **Option**: one trait's choices.
//!
//! Model lists are live: [`Engine::models`] answers from cache and refreshes
//! in the background; the open card polls [`Engine::models_version`]. The
//! picker remembers favorites plus each agent's last model, effort and
//! traits across launches.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::Engine;
use dove_core::harness_detect::Installed;
use dove_core::models;
use dove_core::proto::{HarnessKind, Model, ModelOption, ReasoningLevel};
use dove_core::store::Store;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, Render,
    ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    UniformListScrollHandle, Window, anchored, canvas, deferred, div, px, uniform_list,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::picker_memory::{Pick, PickerMemory};
use crate::theme::{Theme, ink};
use crate::{kit, ui};

pub enum PickerEvent {
    /// The selection (agent, model, effort or a trait) changed.
    Changed,
    /// The popover closed; focus goes back to the prompt.
    Closed,
}

// Zeron's compact-picker geometry.
const WIDTH: f32 = 256.0;
const INSET: f32 = 4.0;
const GAP: f32 = 2.0;
const ROW: f32 = 32.0;
const RADIUS: f32 = 8.0;
const LIST_ROWS: f32 = 7.0;
const SLIDER_H: f32 = 36.0;
const RAIL_H: f32 = 28.0;
const THUMB_W: f32 = 44.0;
const THUMB_H: f32 = 32.0;
/// The slider's width: the card minus its border, inset and the rail's 8pt
/// padding on each side.
const SLIDER_W: f32 = WIDTH - 2.0 - 2.0 * INSET - 16.0;
/// Option ids some agents (Cursor) use for an effort ladder.
const EFFORT_OPTIONS: [&str; 3] = ["effort", "reasoning", "reasoning_effort"];

#[derive(Clone, PartialEq, Eq, Debug)]
enum Page {
    Panel,
    Models,
    /// One trait's choices, by option id.
    Option(String),
}

/// One row of the model list. Rows mix agents.
#[derive(Clone)]
struct Row {
    kind: HarnessKind,
    model: Model,
    /// Another row of the same agent shares this label.
    ambiguous: bool,
}

/// The slider's stops: the model's ladder, else an option shaped like one.
enum Effort {
    Levels(Vec<ReasoningLevel>),
    Option { id: String, choices: Vec<(String, String)> },
}

impl Effort {
    fn labels(&self) -> Vec<String> {
        match self {
            Effort::Levels(levels) => levels.iter().map(|l| l.label().to_string()).collect(),
            Effort::Option { choices, .. } => choices.iter().map(|(_, label)| label.clone()).collect(),
        }
    }
}

pub struct Picker {
    pub harness: HarnessKind,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    pub model_options: Vec<(String, String)>,
    pub open: bool,
    page: Page,
    /// Keyboard cursor into the current page's rows.
    active: usize,
    list_scroll: UniformListScrollHandle,
    /// The model list as last drawn; the virtual list renders from it.
    rows_cache: Rc<Vec<Row>>,
    memory: PickerMemory,
    engine: Option<Arc<Engine>>,
    /// The catalog version the card last drew.
    seen_version: u64,
    installed: Vec<Installed>,
    /// Fixed to one agent (an existing task) rather than choosable.
    locked: bool,
    query: String,
    search: Entity<InputState>,
    focus: FocusHandle,
    /// The slider's last painted bounds, for pointer-to-stop mapping.
    slider: Rc<Cell<Option<Bounds<Pixels>>>>,
    dragging: bool,
    /// A click on the chip right after an outside-click close must not
    /// reopen the card.
    closed_at: Option<Instant>,
    _poll: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PickerEvent> for Picker {}

impl Picker {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search_style = ui::input_style(Theme::of(cx));
        let search = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Search models…");
            s.set_editor_style(search_style);
            s
        });
        let subs = vec![cx.subscribe(&search, |this, s, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.query = s.read(cx).value().to_string();
                this.active = 0;
                this.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
            }
        })];
        let mut this = Self {
            harness: HarnessKind::ClaudeCode,
            model: None,
            reasoning: None,
            model_options: Vec::new(),
            open: false,
            page: Page::Panel,
            active: 0,
            list_scroll: UniformListScrollHandle::new(),
            rows_cache: Rc::default(),
            memory: PickerMemory::default(),
            engine: None,
            seen_version: 0,
            installed: Vec::new(),
            locked: false,
            query: String::new(),
            search,
            focus: cx.focus_handle(),
            slider: Rc::default(),
            dragging: false,
            closed_at: None,
            _poll: None,
            _subs: subs,
        };
        this.ensure_model();
        this
    }

    /// Live catalogs plus the remembered picks.
    pub fn attach_engine(&mut self, engine: Arc<Engine>, cx: &mut Context<Self>) {
        let store = engine.store.clone();
        self.seen_version = engine.models_version();
        self.engine = Some(engine);
        self.attach_memory(store);
        self.start_polling(cx);
    }

    /// Load favorites and remembered picks. The new-session picker also
    /// starts on the agent and model used last.
    pub fn attach_memory(&mut self, store: Arc<Store>) {
        self.memory = PickerMemory::load(store);
        if !self.locked
            && let Some(kind) = self.memory.default_harness()
        {
            self.restore(kind);
        }
    }

    /// Fix the picker to a task's agent. The task's own model wins over the
    /// remembered one, so the chip shows what the session actually runs.
    pub fn lock_to(&mut self, kind: HarnessKind, model: Option<String>) {
        self.harness = kind;
        if let Some(last) = self.memory.last(kind).cloned() {
            self.reasoning = last.reasoning;
            self.model_options = last.options;
            if model.is_none() {
                self.model = Some(last.model);
            }
        }
        if model.is_some() {
            self.model = model;
        }
        self.locked = true;
        self.ensure_model();
    }

    pub fn use_harness(&mut self, kind: HarnessKind, model: Option<String>) {
        if kind != self.harness {
            self.restore(kind);
        }
        if model.is_some() {
            self.model = model;
        }
        self.ensure_model();
    }

    pub fn set_installed(&mut self, installed: Vec<Installed>, cx: &mut Context<Self>) {
        if !self.locked && !installed.iter().any(|i| i.kind == self.harness) {
            if let Some(first) = installed.iter().find(|i| dove_core::harness_supported(i.kind)) {
                self.restore(first.kind);
            }
        }
        self.installed = installed;
        self.ensure_model();
        if let Some(engine) = &self.engine {
            for kind in self.offered() {
                let _ = engine.models(kind);
            }
        }
        cx.emit(PickerEvent::Changed);
        cx.notify();
    }

    fn catalog(&self, kind: HarnessKind) -> Vec<Model> {
        match &self.engine {
            Some(engine) => engine.models(kind),
            None => models::catalog(kind),
        }
    }

    /// Switch to `kind` with the model, effort and traits it had last time.
    fn restore(&mut self, kind: HarnessKind) {
        self.harness = kind;
        let catalog = self.catalog(kind);
        match self.memory.last(kind).filter(|p| catalog.iter().any(|m| m.id == p.model)).cloned() {
            Some(pick) => {
                self.model = Some(pick.model);
                self.reasoning = pick.reasoning;
                self.model_options = pick.options;
            }
            None => self.model = None,
        }
        self.ensure_model();
    }

    fn ensure_model(&mut self) {
        if self.model.is_none() {
            self.model = self.catalog(self.harness).into_iter().next().map(|m| m.id);
        }
        self.align_traits();
    }

    fn align_traits(&mut self) {
        let Some(model) = self.current_model() else {
            return;
        };
        self.reasoning = models::clamp_reasoning(self.reasoning, &model.reasoning_levels);
        self.model_options = models::align_options(&model, &self.model_options);
    }

    fn current_model(&self) -> Option<Model> {
        let id = self.model.as_deref()?;
        self.catalog(self.harness).into_iter().find(|m| m.id == id)
    }

    /// A fresher catalog landed: keep the pick when it still exists. A
    /// new-session pick that vanished falls back to the agent's first model;
    /// a task keeps its own model either way.
    fn on_catalog_changed(&mut self, cx: &mut Context<Self>) {
        if !self.locked && self.model.is_some() && self.current_model().is_none() {
            self.model = None;
        }
        self.ensure_model();
        cx.emit(PickerEvent::Changed);
        cx.notify();
    }

    fn remember(&mut self) {
        if let Some(model) = self.model.clone() {
            let pick = Pick { model, reasoning: self.reasoning, options: self.model_options.clone() };
            self.memory.remember(self.harness, pick, !self.locked);
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.remember();
        cx.emit(PickerEvent::Changed);
        cx.notify();
    }
}

// ------------------------------------------------------------ chip summary

impl Picker {
    fn chip_name(&self) -> String {
        self.current_model()
            .map(|m| m.label)
            .or_else(|| self.model.clone())
            .unwrap_or_else(|| self.harness.label().to_string())
    }

    /// The effort's label, from the ladder or an effort-shaped option.
    fn effort_label(&self, model: &Model) -> Option<String> {
        match effort_of(model)? {
            Effort::Levels(levels) => {
                let current = self.reasoning.or_else(|| models::default_reasoning(&levels))?;
                Some(current.label().to_string())
            }
            Effort::Option { id, choices } => {
                let current = models::option_value(&self.model_options, &id)?;
                choices.iter().find(|(c, _)| c == current).map(|(_, label)| label.clone())
            }
        }
    }

    fn fast_on(&self, model: &Model) -> bool {
        fast_option(model).is_some_and(|(opt, on, _)| models::option_value(&self.model_options, &opt.id) == Some(on))
    }
}

/// The model's effort stops, if it has any.
fn effort_of(model: &Model) -> Option<Effort> {
    if !model.reasoning_levels.is_empty() {
        return Some(Effort::Levels(model.reasoning_levels.clone()));
    }
    let opt = model.options.iter().find(|o| EFFORT_OPTIONS.contains(&o.id.as_str()) && o.choices.len() > 1)?;
    Some(Effort::Option {
        id: opt.id.clone(),
        choices: opt.choices.iter().map(|c| (c.id.clone(), c.label.clone())).collect(),
    })
}

/// The model's fast-mode option, as `(option, on, off)`: an on/off toggle
/// named for speed, or a speed / service tier with a fast choice.
fn fast_option(model: &Model) -> Option<(&ModelOption, &str, &str)> {
    model.options.iter().find_map(|o| {
        if !matches!(o.id.as_str(), "fastMode" | "fast" | "serviceTier" | "speed") || o.choices.len() != 2 {
            return None;
        }
        let on = o.choices.iter().find(|c| matches!(c.id.as_str(), "on" | "true" | "fast" | "priority"))?;
        let off = o.choices.iter().find(|c| c.id != on.id)?;
        Some((o, on.id.as_str(), off.id.as_str()))
    })
}

/// Options the panel draws as its own controls rather than rows.
fn panel_rows(model: &Model) -> Vec<ModelOption> {
    let fast = fast_option(model).map(|(o, _, _)| o.id.clone());
    let effort = match effort_of(model) {
        Some(Effort::Option { id, .. }) => Some(id),
        _ => None,
    };
    model.options.iter().filter(|o| Some(&o.id) != fast.as_ref() && Some(&o.id) != effort.as_ref()).cloned().collect()
}

fn is_toggle(opt: &ModelOption) -> bool {
    opt.choices.len() == 2 && opt.choices.iter().any(|c| c.id == "on") && opt.choices.iter().any(|c| c.id == "off")
}

// ----------------------------------------------------------- open / close

impl Picker {
    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close_menu(window, cx);
            return;
        }
        if self.closed_at.is_some_and(|at| at.elapsed() < Duration::from_millis(250)) {
            return;
        }
        self.open = true;
        self.page = Page::Panel;
        self.active = 0;
        self.dragging = false;
        self.focus.focus(window, cx);
        self.start_polling(cx);
        cx.notify();
    }

    fn close_menu(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.dragging = false;
        self.closed_at = Some(Instant::now());
        cx.emit(PickerEvent::Changed);
        cx.emit(PickerEvent::Closed);
        cx.notify();
    }

    /// While the card is open, redraw when a fresher catalog lands.
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else { return };
        // Kick refreshes for everything the card can show.
        for kind in self.offered() {
            let _ = engine.models(kind);
        }
        if self._poll.is_some() {
            return;
        }
        self._poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(700)).await;
                let version = engine.models_version();
                let _ = this.update(cx, |this, cx| {
                    if version != this.seen_version {
                        this.seen_version = version;
                        this.on_catalog_changed(cx);
                    }
                });
            }
        }));
    }

    fn show_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page = Page::Panel;
        self.active = 0;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn show_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page = Page::Models;
        self.query.clear();
        self.search.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.focus(window, cx);
        });
        let rows = self.rows();
        self.active = rows
            .iter()
            .position(|r| r.kind == self.harness && self.model.as_deref() == Some(r.model.id.as_str()))
            .unwrap_or(0);
        self.list_scroll.scroll_to_item(self.active, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn show_option(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.current_model();
        let current = models::option_value(&self.model_options, &id).map(str::to_string);
        self.active = model
            .as_ref()
            .and_then(|m| m.option(&id))
            .and_then(|o| o.choices.iter().position(|c| Some(&c.id) == current.as_ref()))
            .unwrap_or(0);
        self.page = Page::Option(id);
        self.focus.focus(window, cx);
        cx.notify();
    }
}

// --------------------------------------------------------------- picking

impl Picker {
    /// Agents the card offers: installed and drivable (just the task's own
    /// agent when locked).
    fn offered(&self) -> Vec<HarnessKind> {
        if self.locked {
            return vec![self.harness];
        }
        self.installed.iter().map(|i| i.kind).filter(|k| dove_core::harness_supported(*k)).collect()
    }

    /// The model list: starred first, then one run per agent. A search
    /// spans every offered agent.
    fn rows(&self) -> Vec<Row> {
        let query = self.query.trim().to_ascii_lowercase();
        let offered = self.offered();
        let catalogs: Vec<(HarnessKind, Vec<Model>)> = offered.iter().map(|k| (*k, self.catalog(*k))).collect();
        let matches = |kind: HarnessKind, m: &Model| {
            query.is_empty()
                || [m.label.as_str(), m.id.as_str(), m.description.as_deref().unwrap_or(""), kind.label()]
                    .iter()
                    .any(|field| field.to_ascii_lowercase().contains(&query))
        };
        let mut rows = Vec::new();
        for (kind, id) in self.memory.favorites() {
            if let Some((_, catalog)) = catalogs.iter().find(|(k, _)| k == kind)
                && let Some(m) = catalog.iter().find(|m| &m.id == id)
                && matches(*kind, m)
            {
                rows.push(Row { kind: *kind, model: m.clone(), ambiguous: false });
            }
        }
        for (kind, catalog) in &catalogs {
            for m in catalog {
                if !self.memory.is_favorite(*kind, &m.id) && matches(*kind, m) {
                    rows.push(Row { kind: *kind, model: m.clone(), ambiguous: false });
                }
            }
        }
        let labels: Vec<(HarnessKind, String)> = rows.iter().map(|r| (r.kind, r.model.label.clone())).collect();
        for row in &mut rows {
            row.ambiguous = labels.iter().filter(|(k, l)| *k == row.kind && *l == row.model.label).count() > 1;
        }
        rows
    }

    /// Where each group starts in the unsearched list: `None` is Starred.
    fn groups(&self, rows: &[Row]) -> Vec<(Option<HarnessKind>, usize)> {
        let mut groups: Vec<(Option<HarnessKind>, usize)> = Vec::new();
        for (ix, row) in rows.iter().enumerate() {
            let group = (!self.memory.is_favorite(row.kind, &row.model.id)).then_some(row.kind);
            if groups.last().is_none_or(|(last, _)| *last != group) {
                groups.push((group, ix));
            }
        }
        groups
    }

    fn pick_model(&mut self, kind: HarnessKind, id: String, cx: &mut Context<Self>) {
        if kind != self.harness {
            // A different agent brings back the effort and traits it had.
            if let Some(last) = self.memory.last(kind).cloned() {
                self.reasoning = last.reasoning;
                self.model_options = last.options;
            }
            self.harness = kind;
        }
        self.model = Some(id);
        self.align_traits();
        self.changed(cx);
    }

    /// Tab on the panel: the next agent, at its last-used model.
    fn cycle_agent(&mut self, delta: isize, cx: &mut Context<Self>) {
        let offered = self.offered();
        if self.locked || offered.len() < 2 {
            return;
        }
        let at = offered.iter().position(|k| *k == self.harness).unwrap_or(0) as isize;
        let next = (at + delta).rem_euclid(offered.len() as isize) as usize;
        self.restore(offered[next]);
        self.changed(cx);
    }

    fn set_option(&mut self, id: &str, choice: &str, cx: &mut Context<Self>) {
        self.model_options.retain(|(k, _)| k != id);
        self.model_options.push((id.to_string(), choice.to_string()));
        self.changed(cx);
    }

    fn toggle_fast(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.current_model() else { return };
        let Some((opt, on, off)) = fast_option(&model) else { return };
        let next = if models::option_value(&self.model_options, &opt.id) == Some(on) { off } else { on };
        let id = opt.id.clone();
        let next = next.to_string();
        self.set_option(&id, &next, cx);
    }

    /// Set the effort to stop `index`.
    fn pick_effort(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(model) = self.current_model() else { return };
        match effort_of(&model) {
            Some(Effort::Levels(levels)) => {
                if let Some(level) = levels.get(index).copied()
                    && self.reasoning != Some(level)
                {
                    self.reasoning = Some(level);
                    self.changed(cx);
                }
            }
            Some(Effort::Option { id, choices }) => {
                if let Some((choice, _)) = choices.get(index).cloned()
                    && models::option_value(&self.model_options, &id) != Some(choice.as_str())
                {
                    self.set_option(&id, &choice, cx);
                }
            }
            None => {}
        }
    }

    fn effort_index(&self, model: &Model) -> Option<(usize, usize)> {
        let effort = effort_of(model)?;
        let count = effort.labels().len();
        let at = match effort {
            Effort::Levels(levels) => {
                let current = self.reasoning.or_else(|| models::default_reasoning(&levels));
                levels.iter().position(|l| Some(*l) == current).unwrap_or(0)
            }
            Effort::Option { id, choices } => {
                let current = models::option_value(&self.model_options, &id);
                choices.iter().position(|(c, _)| Some(c.as_str()) == current).unwrap_or(0)
            }
        };
        Some((at, count))
    }

    fn effort_at(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some(bounds) = self.slider.get() else { return };
        let Some(model) = self.current_model() else { return };
        let Some((_, count)) = self.effort_index(&model) else { return };
        if count == 0 {
            return;
        }
        let inset = THUMB_W / 2.0;
        let span = (f32::from(bounds.size.width) - 2.0 * inset).max(1.0);
        let fraction = ((f32::from(x - bounds.left()) - inset) / span).clamp(0.0, 1.0);
        let index = if count > 1 { (fraction * (count - 1) as f32).round() as usize } else { 0 };
        self.pick_effort(index, cx);
    }

    fn toggle_favorite(&mut self, kind: HarnessKind, id: &str, cx: &mut Context<Self>) {
        self.memory.toggle_favorite(kind, id);
        cx.notify();
    }
}

// --------------------------------------------------------------- keyboard

impl Picker {
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let key = ev.keystroke.key.as_str();
        let mods = &ev.keystroke.modifiers;
        match self.page.clone() {
            Page::Panel => match key {
                "escape" => self.close_menu(window, cx),
                "f" if !mods.modified() => self.toggle_fast(cx),
                "up" | "down" => {
                    self.show_models(window, cx);
                    let len = self.rows().len();
                    if len > 0 {
                        self.active = step(self.active, len, if key == "up" { -1 } else { 1 });
                        self.list_scroll.scroll_to_item(self.active, ScrollStrategy::Nearest);
                    }
                }
                "enter" | "space" => self.show_models(window, cx),
                "tab" => self.cycle_agent(if mods.shift { -1 } else { 1 }, cx),
                "left" | "right" | "home" | "end" => {
                    let Some(model) = self.current_model() else { return true };
                    let Some((at, count)) = self.effort_index(&model) else { return true };
                    let last = count.saturating_sub(1);
                    let next = match key {
                        "left" => at.saturating_sub(1),
                        "right" => (at + 1).min(last),
                        "home" => 0,
                        _ => last,
                    };
                    self.pick_effort(next, cx);
                }
                _ => return false,
            },
            Page::Models => {
                let rows = self.rows();
                match key {
                    "escape" => self.show_panel(window, cx),
                    "up" | "down" if !rows.is_empty() => {
                        self.active = step(self.active, rows.len(), if key == "up" { -1 } else { 1 });
                        self.list_scroll.scroll_to_item(self.active, ScrollStrategy::Nearest);
                    }
                    "enter" => {
                        if let Some(row) = rows.get(self.active).cloned() {
                            self.pick_model(row.kind, row.model.id, cx);
                        }
                        self.show_panel(window, cx);
                    }
                    "f" if (mods.control || mods.platform) && mods.shift => {
                        if let Some(row) = rows.get(self.active) {
                            let (kind, id) = (row.kind, row.model.id.clone());
                            self.toggle_favorite(kind, &id, cx);
                        }
                    }
                    "tab" => {}
                    _ => return false,
                }
            }
            Page::Option(id) => {
                let Some(opt) = self.current_model().and_then(|m| m.option(&id).cloned()) else {
                    self.show_panel(window, cx);
                    return true;
                };
                match key {
                    "escape" | "left" => self.show_panel(window, cx),
                    "up" | "down" => {
                        self.active = step(self.active, opt.choices.len(), if key == "up" { -1 } else { 1 })
                    }
                    "enter" | "space" => {
                        if let Some(choice) = opt.choices.get(self.active) {
                            let choice = choice.id.clone();
                            self.set_option(&id, &choice, cx);
                        }
                        self.show_panel(window, cx);
                    }
                    _ => return false,
                }
            }
        }
        cx.notify();
        true
    }
}

fn step(at: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (at as isize + delta).rem_euclid(len as isize) as usize
}

// -------------------------------------------------------------- rendering

impl Picker {
    fn card(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let content = match self.page.clone() {
            Page::Panel => self.panel(t, cx),
            Page::Models => self.models_page(t, cx),
            Page::Option(id) => self.option_page(t, &id, cx),
        };
        let _ = window;
        ui::frosted(
            14.0,
            22.0,
            div()
                .id("agent-picker")
                .track_focus(&self.focus)
                .w(px(WIDTH))
                .rounded(px(14.0))
                .bg(t.glass_overlay())
                .border_1()
                .border_color(t.border_strong)
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::hsla(0.0, 0.0, 0.0, 0.42),
                    offset: gpui::point(px(0.0), px(16.0)),
                    blur_radius: px(40.0),
                    spread_radius: px(0.0),
                    inset: false,
                }])
                .overflow_hidden()
                // Blocks clicks from reaching what sits under the card. The
                // outside-click close lives on this same element: occluding
                // a child would make every inside click look like outside.
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_menu(window, cx)))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        if this.dragging {
                            this.dragging = false;
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        if this.dragging {
                            this.dragging = false;
                            cx.notify();
                        }
                    }),
                )
                .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    if this.on_key(ev, window, cx) {
                        cx.stop_propagation();
                    }
                }))
                .child(content),
        )
        .into_any_element()
    }

    // ---- panel

    fn panel(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(model) = self.current_model() else {
            let note = if self.offered().is_empty() { "No agents on PATH" } else { "Select model" };
            return div().p(px(INSET)).child(self.title_button(t, None, note.to_string(), cx)).into_any_element();
        };
        let effort = self.effort_label(&model);
        let mut header = div()
            .h(px(if effort.is_some() { 48.0 } else { 36.0 }))
            .flex()
            .gap(px(INSET))
            .child(self.title_button(t, effort, model.label.clone(), cx));
        if let Some((opt, on, _)) = fast_option(&model) {
            let fast = models::option_value(&self.model_options, &opt.id) == Some(on);
            header = header.child(
                div()
                    .id("picker-fast")
                    .w(px(32.0))
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(RADIUS))
                    .cursor_pointer()
                    .hover(|s| s.bg(ink(0.05)))
                    .tooltip(ui::tip(if fast {
                        "Fast mode on · F to turn off"
                    } else {
                        "Fast mode off · F to turn on"
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_fast(cx)))
                    .child(
                        icon(if fast { icons::FAST_TIER_BOLD } else { icons::FAST_TIER })
                            .size(px(15.0))
                            .text_color(if fast { t.accent } else { t.text_muted }),
                    ),
            );
        }
        let mut panel = div().p(px(INSET)).flex().flex_col().child(header);
        if let Some((at, count)) = self.effort_index(&model) {
            panel = panel.child(div().px(px(8.0)).pt(px(2.0)).pb(px(4.0)).child(self.slider(
                t,
                at,
                count,
                self.fast_on(&model),
                cx,
            )));
        }
        let rows = panel_rows(&model);
        if !rows.is_empty() {
            let mut list = div().mt(px(4.0)).pt(px(2.0)).flex().flex_col().gap(px(GAP));
            for opt in rows {
                list = list.child(self.option_row(t, &opt, cx));
            }
            panel = panel.child(list);
        }
        panel.into_any_element()
    }

    /// The effort over the model's name; opens the model list.
    fn title_button(&self, t: &Theme, effort: Option<String>, name: String, cx: &mut Context<Self>) -> AnyElement {
        let two_lines = effort.is_some();
        div()
            .id("picker-title")
            .group("picker-title")
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h(px(36.0))
            .px(px(8.0))
            .flex()
            .flex_col()
            .justify_center()
            .rounded(px(RADIUS))
            .cursor_pointer()
            .hover(|s| s.bg(ink(0.05)))
            .on_click(cx.listener(|this, _, window, cx| this.show_models(window, cx)))
            .when_some(effort, |d, effort| {
                d.child(
                    div()
                        .text_size(px(14.0))
                        .line_height(px(17.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(t.text)
                        .child(effort),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .min_w_0()
                    .map(|d| {
                        if two_lines {
                            d.text_size(px(12.0)).line_height(px(15.0)).text_color(t.text_muted)
                        } else {
                            d.text_size(px(14.0))
                                .line_height(px(17.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(t.text)
                        }
                    })
                    .child(div().min_w_0().truncate().child(name))
                    .child(
                        icon(icons::ALT_ARROW_RIGHT)
                            .size(px(10.0))
                            .flex_none()
                            .text_color(t.text_muted)
                            .opacity(0.6)
                            .group_hover("picker-title", |s| s.opacity(1.0)),
                    ),
            )
            .into_any_element()
    }

    /// A pill rail with one dot per stop, an accent fill up to a glass thumb.
    fn slider(&self, t: &Theme, at: usize, count: usize, fast: bool, cx: &mut Context<Self>) -> AnyElement {
        let inset = THUMB_W / 2.0;
        let span = SLIDER_W - 2.0 * inset;
        let fraction = if count > 1 { at as f32 / (count - 1) as f32 } else { 0.0 };
        let center = inset + fraction * span;
        let rail_top = (SLIDER_H - RAIL_H) / 2.0;
        let bounds = self.slider.clone();
        let mut slider = div()
            .id("picker-effort")
            .relative()
            .w(px(SLIDER_W))
            .h(px(SLIDER_H))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                    this.dragging = true;
                    this.effort_at(ev.position.x, cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if this.dragging && ev.pressed_button == Some(MouseButton::Left) {
                    this.effort_at(ev.position.x, cx);
                }
            }))
            .child(canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {}).absolute().inset_0())
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(rail_top))
                    .h(px(RAIL_H))
                    .rounded_full()
                    .bg(ink(0.06))
                    .border_1()
                    .border_color(ink(0.07)),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(rail_top))
                    .h(px(RAIL_H))
                    .w(px(center + RAIL_H / 2.0))
                    .rounded_full()
                    .bg(t.accent.opacity(if fast { 0.95 } else { 0.8 }))
                    .when(fast, |d| {
                        d.shadow(vec![gpui::BoxShadow {
                            color: t.accent.opacity(0.45),
                            offset: gpui::point(px(0.0), px(0.0)),
                            blur_radius: px(10.0),
                            spread_radius: px(0.0),
                            inset: false,
                        }])
                    }),
            );
        for i in 0..count {
            if i == at {
                continue;
            }
            let x = inset + if count > 1 { i as f32 / (count - 1) as f32 * span } else { 0.0 };
            let filled = i < at;
            slider = slider.child(
                div()
                    .absolute()
                    .left(px(x - 2.0))
                    .top(px(SLIDER_H / 2.0 - 2.0))
                    .size(px(4.0))
                    .rounded_full()
                    .bg(if filled { gpui::hsla(0.0, 0.0, 1.0, 0.7) } else { ink(0.28) }),
            );
        }
        slider
            .child(
                div()
                    .absolute()
                    .left(px(center - THUMB_W / 2.0))
                    .top(px((SLIDER_H - THUMB_H) / 2.0))
                    .w(px(THUMB_W))
                    .h(px(THUMB_H))
                    .rounded_full()
                    .bg(gpui::hsla(0.0, 0.0, 0.98, 1.0))
                    .border_1()
                    .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.08))
                    .shadow(vec![gpui::BoxShadow {
                        color: gpui::hsla(0.0, 0.0, 0.0, if self.dragging { 0.30 } else { 0.22 }),
                        offset: gpui::point(px(0.0), px(if self.dragging { 3.0 } else { 1.0 })),
                        blur_radius: px(if self.dragging { 10.0 } else { 4.0 }),
                        spread_radius: px(0.0),
                        inset: false,
                    }]),
            )
            .into_any_element()
    }

    /// A trait row: a switch for on/off, else the choice and a chevron.
    fn option_row(&self, t: &Theme, opt: &ModelOption, cx: &mut Context<Self>) -> AnyElement {
        let current = models::option_value(&self.model_options, &opt.id).unwrap_or(&opt.default_choice).to_string();
        let row = div()
            .id(SharedString::from(format!("picker-opt-{}", opt.id)))
            .h(px(26.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(RADIUS))
            .cursor_pointer()
            .hover(|s| s.bg(ink(0.05)))
            .text_size(px(12.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text)
                    .child(opt.label.clone()),
            );
        let id = opt.id.clone();
        if is_toggle(opt) {
            let on = current == "on";
            return row
                .child(kit::switch(SharedString::from(format!("picker-switch-{}", opt.id)), on, t))
                .on_click(cx.listener(move |this, _, _, cx| this.set_option(&id, if on { "off" } else { "on" }, cx)))
                .into_any_element();
        }
        let label = opt.choices.iter().find(|c| c.id == current).map(|c| c.label.clone()).unwrap_or(current);
        row.child(div().flex_none().text_color(t.text_muted).child(label))
            .child(icon(icons::ALT_ARROW_RIGHT).size(px(10.0)).text_color(t.text_muted))
            .on_click(cx.listener(move |this, _, window, cx| this.show_option(id.clone(), window, cx)))
            .into_any_element()
    }

    // ---- model list

    fn back_button(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("picker-back")
            .flex_none()
            .size(px(ROW))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(RADIUS))
            .cursor_pointer()
            .hover(|s| s.bg(ink(0.05)))
            .tooltip(ui::tip("Back (Esc)"))
            .on_click(cx.listener(|this, _, window, cx| this.show_panel(window, cx)))
            .child(icon(icons::ALT_ARROW_LEFT).size(px(14.0)).text_color(t.text_muted))
            .into_any_element()
    }

    fn models_page(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.rows();
        let searching = !self.query.trim().is_empty();
        let groups = if searching || self.locked { Vec::new() } else { self.groups(&rows) };
        // The group holding the highlighted row lights its tab.
        let current = groups.iter().rev().find(|(_, start)| *start <= self.active).map(|(g, _)| *g);

        let mut strip = div()
            .id("picker-groups")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap(px(GAP))
            .overflow_x_scroll();
        for (group, start) in &groups {
            let (glyph, tint, name) = match group {
                None => (icons::STAR_BOLD, t.text_muted, "Starred".to_string()),
                Some(kind) => {
                    (ui::harness_icon(*kind), ui::harness_tint(*kind, t, t.text_muted), kind.label().to_string())
                }
            };
            let viewed = current == Some(*group);
            let start = *start;
            strip = strip.child(
                div()
                    .id(SharedString::from(format!("picker-group-{start}")))
                    .flex_none()
                    .size(px(ROW))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(RADIUS))
                    .cursor_pointer()
                    .when(viewed, |d| d.bg(ink(0.08)))
                    .when(!viewed, |d| d.opacity(0.7).hover(|s| s.bg(ink(0.05)).opacity(1.0)))
                    .tooltip(ui::tip(name))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active = start;
                        this.list_scroll.scroll_to_item_strict(start, ScrollStrategy::Top);
                        cx.notify();
                    }))
                    .child(icon(glyph).size(px(14.0)).text_color(tint)),
            );
        }
        let header = div()
            .h(px(ROW + 2.0 * INSET))
            .flex_none()
            .px(px(INSET))
            .flex()
            .items_center()
            .gap(px(GAP))
            .child(self.back_button(t, cx))
            .when(groups.is_empty(), |d| {
                d.child(
                    div()
                        .pl(px(4.0))
                        .text_size(px(13.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.text)
                        .child("Models"),
                )
            })
            .child(strip);
        let search = div()
            .h(px(40.0))
            .flex_none()
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .border_b_1()
            .border_color(t.border)
            .text_size(px(13.0))
            .child(icon(icons::MAGNIFER).size(px(14.0)).text_color(t.text_muted).flex_none())
            .child(div().flex_1().min_w_0().child(Input::new(&self.search)));

        let visible = (rows.len() as f32).clamp(4.0, LIST_ROWS);
        let count = rows.len();
        self.rows_cache = Rc::new(rows);
        // Hundreds of rows (Kilo alone lists 300+): draw only those in view.
        let list: AnyElement = if count == 0 {
            let note = if searching { "Nothing matches that search" } else { "No models yet" };
            div()
                .h(px(visible * (ROW + GAP) + 2.0 * INSET))
                .px(px(12.0))
                .py(px(10.0))
                .text_size(px(12.0))
                .text_color(t.text_muted)
                .child(note)
                .into_any_element()
        } else {
            uniform_list(
                "picker-models",
                count,
                cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                    let t = Theme::of(cx).clone();
                    let rows = this.rows_cache.clone();
                    range.filter_map(|ix| rows.get(ix).map(|row| this.model_row(&t, ix, row, cx))).collect::<Vec<_>>()
                }),
            )
            .h(px(visible * (ROW + GAP) + 2.0 * INSET))
            .p(px(INSET))
            .track_scroll(&self.list_scroll)
            .into_any_element()
        };
        div().flex().flex_col().child(header).child(search).child(list).into_any_element()
    }

    fn model_row(&self, t: &Theme, ix: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let kind = row.kind;
        let id = row.model.id.clone();
        let selected = kind == self.harness && self.model.as_deref() == Some(id.as_str());
        let favorite = self.memory.is_favorite(kind, &id);
        let active = ix == self.active;
        let attribution = row.model.description.clone().filter(|d| row.ambiguous && !d.trim().is_empty());
        let details = match &row.model.description {
            Some(d) => format!("{} · {}\n{d}", row.model.label, kind.label()),
            None => format!("{} · {}", row.model.label, kind.label()),
        };
        let star_id = id.clone();
        div()
            .pb(px(GAP))
            .child(
                div()
                    .id(SharedString::from(format!("picker-row-{ix}")))
                    .h(px(ROW))
                    .pl(px(8.0))
                    .pr(px(4.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(RADIUS))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(ink(0.09)))
                    .when(!selected && active, |d| d.bg(ink(0.05)))
                    .tooltip(ui::tip(details))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.active != ix {
                            this.active = ix;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.pick_model(kind, id.clone(), cx);
                        this.show_panel(window, cx);
                    }))
                    .child(icon(ui::harness_icon(kind)).size(px(14.0)).flex_none().text_color(ui::harness_tint(
                        kind,
                        t,
                        t.text_muted,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_baseline()
                            .gap(px(6.0))
                            .text_size(px(12.0))
                            .child(
                                div()
                                    .flex_none()
                                    .max_w_full()
                                    .truncate()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(t.text)
                                    .child(row.model.label.clone()),
                            )
                            .when_some(attribution, |d, a| {
                                d.child(
                                    div().min_w_0().truncate().text_size(px(11.0)).text_color(t.text_muted).child(a),
                                )
                            }),
                    )
                    .when(selected, |d| d.child(icon(icons::CHECK).size(px(13.0)).flex_none().text_color(t.accent)))
                    .child(
                        div()
                            .id(SharedString::from(format!("picker-star-{ix}")))
                            .size(px(24.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .when(!favorite && !active, |d| d.opacity(0.0))
                            .hover(|s| s.bg(ink(0.08)))
                            .tooltip(ui::tip(if favorite {
                                "Remove from favorites · Ctrl+Shift+F"
                            } else {
                                "Add to favorites · Ctrl+Shift+F"
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_favorite(kind, &star_id, cx);
                            }))
                            .child(
                                icon(if favorite { icons::STAR_BOLD } else { icons::STAR })
                                    .size(px(13.0))
                                    .text_color(if favorite { t.accent } else { t.text_muted }),
                            ),
                    ),
            )
            .into_any_element()
    }

    // ---- one trait's choices

    fn option_page(&mut self, t: &Theme, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(opt) = self.current_model().and_then(|m| m.option(id).cloned()) else {
            return div().into_any_element();
        };
        let current = models::option_value(&self.model_options, id).unwrap_or(&opt.default_choice).to_string();
        let header = div()
            .h(px(ROW + 2.0 * INSET))
            .px(px(INSET))
            .flex()
            .items_center()
            .gap(px(GAP))
            .child(self.back_button(t, cx))
            .child(
                div()
                    .pl(px(4.0))
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text)
                    .child(opt.label.clone()),
            );
        let mut list = div().px(px(INSET)).pb(px(INSET)).flex().flex_col().gap(px(GAP));
        for (ix, choice) in opt.choices.iter().enumerate() {
            let on = choice.id == current;
            let (opt_id, choice_id) = (opt.id.clone(), choice.id.clone());
            list = list.child(
                div()
                    .id(SharedString::from(format!("picker-choice-{ix}")))
                    .h(px(ROW))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(RADIUS))
                    .cursor_pointer()
                    .text_size(px(12.0))
                    .when(on, |d| d.bg(ink(0.09)))
                    .when(!on && ix == self.active, |d| d.bg(ink(0.05)))
                    .hover(|s| s.bg(ink(0.05)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_option(&opt_id, &choice_id, cx);
                        this.show_panel(window, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child(choice.label.clone()),
                    )
                    .when(on, |d| d.child(icon(icons::CHECK).size(px(13.0)).text_color(t.accent))),
            );
        }
        div().flex().flex_col().child(header).child(list).into_any_element()
    }
}

impl Render for Picker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let t = &theme;
        let kind = self.harness;
        let model = self.current_model();
        let effort = model.as_ref().and_then(|m| self.effort_label(m));
        let fast = model.as_ref().is_some_and(|m| self.fast_on(m));
        let trigger = div()
            .id("composer-agent")
            .h(px(28.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(px(13.0))
            .text_color(t.text)
            .hover(|s| s.bg(t.wash(0.08)))
            .child(icon(ui::harness_icon(kind)).size(px(14.0)).text_color(ui::harness_tint(kind, t, t.text_muted)))
            .child(div().truncate().max_w(px(160.0)).child(self.chip_name()))
            .when_some(effort, |d, effort| d.child(div().text_color(t.text_faint).child(effort)))
            .when(fast, |d| d.child(icon(icons::FAST_TIER).size(px(13.0)).text_color(t.accent)))
            .tooltip(ui::tip(if self.locked { "Model and effort" } else { "Agent, model and effort" }))
            .on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx)));
        let card = self.open.then(|| self.card(t, window, cx));
        div().relative().child(trigger).when_some(card, |d, card| {
            d.child(deferred(
                anchored()
                    .anchor(Anchor::BottomLeft)
                    .snap_to_window_with_margin(px(8.0))
                    .child(div().mb(px(34.0)).child(card)),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dove_core::proto::ModelOptionChoice;

    fn opt(id: &str, choices: &[&str], default: &str) -> ModelOption {
        ModelOption {
            id: id.into(),
            label: id.into(),
            choices: choices.iter().map(|c| ModelOptionChoice { id: (*c).into(), label: (*c).into() }).collect(),
            default_choice: default.into(),
        }
    }

    fn model(levels: &[ReasoningLevel], options: Vec<ModelOption>) -> Model {
        Model { id: "m".into(), label: "M".into(), description: None, reasoning_levels: levels.to_vec(), options }
    }

    #[test]
    fn fast_mode_is_found_in_every_shape() {
        for (o, on) in [
            (opt("fastMode", &["off", "on"], "off"), "on"),
            (opt("serviceTier", &["default", "fast"], "default"), "fast"),
            (opt("speed", &["standard", "fast"], "standard"), "fast"),
        ] {
            let m = model(&[], vec![o]);
            assert_eq!(fast_option(&m).map(|(_, on, _)| on), Some(on));
            assert!(panel_rows(&m).is_empty(), "the fast option is the button, not a row");
        }
        assert!(fast_option(&model(&[], vec![opt("contextWindow", &["200k", "1m"], "200k")])).is_none());
    }

    #[test]
    fn effort_comes_from_the_ladder_or_an_effort_option() {
        let m = model(&[ReasoningLevel::Low, ReasoningLevel::High], vec![]);
        assert!(matches!(effort_of(&m), Some(Effort::Levels(l)) if l.len() == 2));
        let cursor = model(&[], vec![opt("reasoning", &["low", "medium", "high"], "medium")]);
        assert!(matches!(effort_of(&cursor), Some(Effort::Option { ref id, .. }) if id == "reasoning"));
        assert!(panel_rows(&cursor).is_empty(), "the effort option is the slider, not a row");
        assert!(effort_of(&model(&[], vec![])).is_none());
    }

    #[test]
    fn step_wraps() {
        assert_eq!(step(0, 3, -1), 2);
        assert_eq!(step(2, 3, 1), 0);
        assert_eq!(step(0, 0, 1), 0);
    }
}
