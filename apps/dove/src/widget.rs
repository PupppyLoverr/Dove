//! Generative UI: interactive cards an agent shows inline in the chat.
//!
//! An agent emits a small JSON spec, either as a ` ```ui ` fence in its
//! reply or through the `show_ui` tool of Dove's MCP server. The card can
//! hold text, callouts, stat tiles, bar and line charts, tables, progress,
//! key/value lists, code and images, plus controls the user can change
//! (choices, toggles, number steppers, checklists, sliders, ratings, quick
//! replies). It can also lay out designs natively: donut charts, metric
//! sparklines, colour palettes, galleries, boards, and a `mockup` frame that
//! puts any blocks inside a phone or browser shell. Pressing an action sends
//! the card's values back as an ordinary message, so any harness can keep
//! going; "Ask to change" puts a request to alter the card in the composer.

use std::rc::Rc;

use gpui::StyledImage as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, PathBuilder, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, canvas, div, point, px,
};
use serde::Deserialize;
use serde_json::Value;

use crate::assets::{icon, icons};
use crate::theme::Theme;
use crate::ui;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Spec {
    /// Stable id: showing a card with the same id again updates it in place.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// v1 choice fields (kept for older specs).
    #[serde(default)]
    pub fields: Vec<Field>,
    #[serde(default)]
    pub blocks: Vec<Item>,
    #[serde(default)]
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Field {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Action {
    #[serde(default)]
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Stat {
    pub label: String,
    #[serde(deserialize_with = "any_string")]
    pub value: String,
    #[serde(default)]
    pub delta: String,
    /// A small trend line under the value.
    #[serde(default)]
    pub spark: Vec<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Swatch {
    #[serde(default)]
    pub name: String,
    pub hex: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Shot {
    pub src: String,
    #[serde(default)]
    pub caption: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Column {
    pub title: String,
    #[serde(default)]
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Reply {
    #[serde(default)]
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Datum {
    pub label: String,
    pub value: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    pub title: String,
    #[serde(default)]
    pub detail: String,
    /// done | active | todo | failed
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OptionCard {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub badge: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pane {
    pub label: String,
    #[serde(default)]
    pub blocks: Vec<Item>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Check {
    pub label: String,
    #[serde(default)]
    pub checked: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PricingPlan {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default, deserialize_with = "any_string")]
    pub price: String,
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub features: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Item {
    Text {
        text: String,
    },
    Callout {
        #[serde(default)]
        tone: String,
        #[serde(default)]
        title: String,
        text: String,
    },
    Stats {
        items: Vec<Stat>,
    },
    Bars {
        #[serde(default)]
        title: String,
        items: Vec<Datum>,
        #[serde(default)]
        unit: String,
        #[serde(default)]
        horizontal: bool,
    },
    Line {
        #[serde(default)]
        title: String,
        values: Vec<f64>,
        #[serde(default)]
        labels: Vec<String>,
        #[serde(default)]
        unit: String,
    },
    Table {
        #[serde(default)]
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
    Progress {
        #[serde(default)]
        label: String,
        value: f64,
    },
    Kv {
        items: Vec<(String, Value)>,
    },
    Code {
        #[serde(default)]
        lang: String,
        text: String,
    },
    Image {
        src: String,
        #[serde(default)]
        alt: String,
    },
    Checklist {
        id: String,
        #[serde(default)]
        label: String,
        items: Vec<Check>,
    },
    Toggle {
        id: String,
        label: String,
        #[serde(default)]
        value: bool,
    },
    Number {
        id: String,
        label: String,
        #[serde(default)]
        value: f64,
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
    },
    Choice {
        id: String,
        #[serde(default)]
        label: String,
        choices: Vec<String>,
        value: Option<String>,
    },
    Divider,
    Timeline {
        #[serde(default)]
        title: String,
        steps: Vec<Step>,
    },
    /// Selectable option cards (pick one).
    Cards {
        id: String,
        #[serde(default)]
        label: String,
        items: Vec<OptionCard>,
        value: Option<String>,
    },
    /// A 100% stacked bar with a legend.
    Breakdown {
        #[serde(default)]
        title: String,
        items: Vec<Datum>,
        #[serde(default)]
        unit: String,
    },
    /// Switchable sections; the selected tab is the control's value.
    Tabs {
        id: String,
        tabs: Vec<Pane>,
    },
    /// A section heading inside a card.
    Heading {
        text: String,
    },
    /// A ring chart with a legend.
    Donut {
        #[serde(default)]
        title: String,
        items: Vec<Datum>,
        #[serde(default)]
        unit: String,
    },
    /// Colours with names and hex codes (a palette, a theme).
    Swatches {
        #[serde(default)]
        title: String,
        items: Vec<Swatch>,
    },
    /// Images in a grid.
    Gallery {
        items: Vec<Shot>,
    },
    /// Columns of cards (a board, a roadmap).
    Board {
        columns: Vec<Column>,
    },
    /// Buttons that answer straight away (quick replies).
    Replies {
        #[serde(default)]
        label: String,
        items: Vec<Reply>,
    },
    /// A value on a track.
    Slider {
        id: String,
        label: String,
        #[serde(default)]
        value: f64,
        #[serde(default)]
        min: f64,
        #[serde(default = "hundred")]
        max: f64,
        #[serde(default)]
        step: Option<f64>,
        #[serde(default)]
        unit: String,
    },
    /// Stars out of `max`.
    Rating {
        id: String,
        label: String,
        #[serde(default)]
        value: u32,
        #[serde(default = "five")]
        max: u32,
    },
    /// A link out (docs, a deploy, a PR).
    Link {
        label: String,
        url: String,
    },
    /// A device frame around blocks: an app screen or a web page, natively.
    Mockup {
        /// `phone` (default), `tablet`, `browser`, or `app`.
        #[serde(default)]
        device: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        nav: Vec<String>,
        blocks: Vec<Item>,
    },
    /// A hero section for web or app landing designs.
    Hero {
        #[serde(default)]
        badge: String,
        title: String,
        #[serde(default)]
        subtitle: String,
        #[serde(default)]
        cta: String,
        #[serde(default)]
        secondary: String,
    },
    /// An interactive navigation bar for app or web designs.
    Navbar {
        #[serde(default)]
        id: String,
        #[serde(default)]
        brand: String,
        #[serde(default)]
        items: Vec<String>,
        #[serde(default)]
        cta: String,
    },
    /// Interactive pricing tier cards (pick a plan).
    Pricing {
        #[serde(default)]
        id: String,
        #[serde(default)]
        plans: Vec<PricingPlan>,
    },
    /// Two-column split layout for side-by-side app or dashboard sections.
    Split {
        #[serde(default)]
        left: Vec<Item>,
        #[serde(default)]
        right: Vec<Item>,
    },
    #[serde(other)]
    Unknown,
}

fn hundred() -> f64 {
    100.0
}

fn five() -> u32 {
    5
}

fn any_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        other => other.to_string(),
    })
}

fn cell(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn number(v: f64) -> String {
    if v.fract().abs() < 1e-9 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

impl Spec {
    /// A spec from JSON text (a ` ```ui ` fence).
    pub fn parse(src: &str) -> Option<Self> {
        Self::from_value(serde_json::from_str(src.trim()).ok()?)
    }

    /// A spec from a JSON value (the `show_ui` tool's input).
    pub fn from_value(v: Value) -> Option<Self> {
        let v: Spec = serde_json::from_value(v).ok()?;
        if v.title.is_empty() && v.body.is_empty() && v.actions.is_empty() && v.fields.is_empty() && v.blocks.is_empty()
        {
            return None;
        }
        Some(v)
    }

    fn label_of(&self, id: &str) -> String {
        for b in &self.blocks {
            match b {
                Item::Checklist { id: i, label, .. }
                | Item::Toggle { id: i, label, .. }
                | Item::Number { id: i, label, .. }
                | Item::Choice { id: i, label, .. }
                | Item::Cards { id: i, label, .. }
                | Item::Slider { id: i, label, .. }
                | Item::Rating { id: i, label, .. }
                    if i == id && !label.is_empty() =>
                {
                    return label.clone();
                }
                _ => {}
            }
        }
        self.fields
            .iter()
            .find(|f| f.id == id && !f.label.is_empty())
            .map_or_else(|| id.to_string(), |f| f.label.clone())
    }

    /// The message sent when the user presses `action`.
    pub fn reply(&self, action: &Action, values: &[(String, String)]) -> String {
        let mut out = if self.title.is_empty() {
            format!("I chose \"{}\".", action.label)
        } else {
            format!("On \"{}\" I chose \"{}\".", self.title, action.label)
        };
        for (id, v) in values {
            out.push_str(&format!("\n- {}: {}", self.label_of(id), if v.is_empty() { "(none)" } else { v }));
        }
        out
    }
}

/// One live card: the user's current values and the last action sent.
#[derive(Clone)]
pub struct State {
    pub spec: Spec,
    pub values: Vec<(String, String)>,
    pub sent: Option<String>,
}

impl State {
    pub fn new(spec: Spec) -> Self {
        let mut values: Vec<(String, String)> =
            spec.fields.iter().filter_map(|f| f.choices.first().cloned().map(|c| (f.id.clone(), c))).collect();
        collect_values(&spec.blocks, &mut values);
        Self { spec, values, sent: None }
    }

    pub fn get(&self, id: &str) -> &str {
        self.values.iter().find(|(k, _)| k == id).map_or("", |(_, v)| v.as_str())
    }

    pub fn set(&mut self, id: &str, value: &str) {
        if let Some((_, v)) = self.values.iter_mut().find(|(k, _)| k == id) {
            *v = value.to_string();
        } else {
            self.values.push((id.to_string(), value.to_string()));
        }
    }

    fn checked(&self, id: &str) -> Vec<String> {
        self.get(id).split(", ").filter(|s| !s.is_empty()).map(str::to_string).collect()
    }
}

fn collect_values(blocks: &[Item], values: &mut Vec<(String, String)>) {
    for b in blocks {
        match b {
            Item::Checklist { id, items, .. } => values.push((
                id.clone(),
                items.iter().filter(|c| c.checked).map(|c| c.label.clone()).collect::<Vec<_>>().join(", "),
            )),
            Item::Toggle { id, value, .. } => values.push((id.clone(), if *value { "on" } else { "off" }.into())),
            Item::Number { id, value, .. } => values.push((id.clone(), number(*value))),
            Item::Choice { id, choices, value, .. } => {
                values.push((id.clone(), value.clone().or_else(|| choices.first().cloned()).unwrap_or_default()))
            }
            Item::Cards { id, items, value, .. } => values.push((
                id.clone(),
                value.clone().or_else(|| items.first().map(|c| c.title.clone())).unwrap_or_default(),
            )),
            Item::Tabs { id, tabs } => {
                values.push((id.clone(), tabs.first().map(|p| p.label.clone()).unwrap_or_default()));
                for pane in tabs {
                    collect_values(&pane.blocks, values);
                }
            }
            Item::Slider { id, value, min, max, .. } => values.push((id.clone(), number(value.clamp(*min, *max)))),
            Item::Rating { id, value, .. } => values.push((id.clone(), value.to_string())),
            Item::Navbar { id, items, .. } if !id.is_empty() => {
                values.push((id.clone(), items.first().cloned().unwrap_or_default()))
            }
            Item::Pricing { id, plans } if !id.is_empty() => {
                values.push((id.clone(), plans.first().map(|p| p.name.clone()).unwrap_or_default()))
            }
            Item::Split { left, right } => {
                collect_values(left, values);
                collect_values(right, values);
            }
            Item::Mockup { blocks, .. } => collect_values(blocks, values),
            _ => {}
        }
    }
}

/// What the card asks its host to do.
pub enum Event {
    /// Send this message to the agent (an action was pressed).
    Act { action: String, message: String },
    /// A control changed: `(id, new value)`.
    Set { id: String, value: String },
    /// Put "change this card: …" in the composer.
    Edit(String),
}

type Handler<V> = Rc<dyn Fn(&mut V, Event, &mut Window, &mut Context<V>)>;

fn emit<V: 'static>(weak: &WeakEntity<V>, handler: &Handler<V>, ev: Event, window: &mut Window, cx: &mut gpui::App) {
    let handler = handler.clone();
    let _ = weak.update(cx, |this, cx| handler(this, ev, window, cx));
}

pub fn render<V: 'static>(
    id: impl Into<SharedString>,
    state: &State,
    theme: &Theme,
    weak: WeakEntity<V>,
    handler: impl Fn(&mut V, Event, &mut Window, &mut Context<V>) + 'static,
) -> AnyElement {
    let handler: Handler<V> = Rc::new(handler);
    let id: SharedString = id.into();
    let t = theme;
    let spec = &state.spec;
    let mut card = div()
        .id(id.clone())
        .group("gen-card")
        .w_full()
        .p(px(16.0))
        .rounded(px(16.0))
        .border_1()
        .border_color(t.border)
        .bg(t.surface_card)
        .shadow(vec![gpui::BoxShadow {
            color: gpui::black().opacity(0.06),
            offset: point(px(0.0), px(2.0)),
            blur_radius: px(10.0),
            spread_radius: px(0.0),
            inset: false,
        }])
        .flex()
        .flex_col()
        .gap(px(14.0));

    // Header: title + "Ask to change".
    let title = if spec.title.is_empty() { "Interactive card".to_string() } else { spec.title.clone() };
    let edit_title = title.clone();
    let (w, h) = (weak.clone(), handler.clone());
    card = card.child(
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .size(px(22.0))
                    .flex_none()
                    .rounded(px(7.0))
                    .bg(t.wash(0.06))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(icons::WIDGET).size(px(13.0)).text_color(t.text_muted)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(14.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(title),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{id}-edit")))
                    .flex_none()
                    .h(px(24.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .rounded(px(7.0))
                    .text_size(px(11.5))
                    .text_color(t.text_faint)
                    .cursor_pointer()
                    .invisible()
                    .group_hover("gen-card", |s| s.visible())
                    .hover(|s| s.bg(t.wash(0.08)).text_color(t.text))
                    .role(gpui::Role::Button)
                    .aria_label("Ask the agent to change this card")
                    .tooltip(ui::tip("Ask the agent to change this card"))
                    .child(icon(icons::PEN).size(px(11.0)).text_color(t.text_faint))
                    .child("Change")
                    .on_click(move |_, window, cx| {
                        emit(&w, &h, Event::Edit(format!("Change the \"{edit_title}\" card: ")), window, cx)
                    }),
            ),
    );
    if !spec.body.is_empty() {
        card = card
            .child(div().text_size(px(13.5)).line_height(px(20.0)).text_color(t.text_muted).child(spec.body.clone()));
    }

    for (n, item) in spec.blocks.iter().enumerate() {
        let key = format!("{id}-b{n}");
        card = card.child(render_item(&key, item, state, t, &weak, &handler));
    }

    // v1 fields.
    for field in &spec.fields {
        let label = if field.label.is_empty() { field.id.clone() } else { field.label.clone() };
        card = card.child(choice_row(
            &format!("{id}-f-{}", field.id),
            &field.id,
            &label,
            &field.choices,
            state,
            t,
            &weak,
            &handler,
        ));
    }

    if !spec.actions.is_empty() {
        let mut row = div().flex().flex_wrap().items_center().gap(px(8.0)).pt(px(2.0));
        for (i, action) in spec.actions.iter().enumerate() {
            let aid = if action.id.is_empty() { action.label.clone() } else { action.id.clone() };
            let message = spec.reply(action, &state.values);
            let el_id = SharedString::from(format!("{id}-a-{i}"));
            let btn = if i == 0 {
                ui::button_primary(el_id, action.label.clone(), t)
            } else {
                ui::button_ghost(el_id, action.label.clone(), t)
            };
            let (w, h) = (weak.clone(), handler.clone());
            let a = aid.clone();
            row = row.child(btn.on_click(move |_, window, cx| {
                emit(&w, &h, Event::Act { action: a.clone(), message: message.clone() }, window, cx)
            }));
        }
        if let Some(sent) = &state.sent {
            let label = spec
                .actions
                .iter()
                .find(|a| (if a.id.is_empty() { &a.label } else { &a.id }) == sent)
                .map_or(sent.clone(), |a| a.label.clone());
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .text_size(px(12.0))
                    .text_color(t.success)
                    .child(icon(icons::CHECK).size(px(12.0)).text_color(t.success))
                    .child(format!("Sent \"{label}\"")),
            );
        }
        card = card.child(row);
    }
    card.into_any_element()
}

fn section_title(text: &str, t: &Theme) -> gpui::Div {
    div().text_size(px(11.5)).font_weight(gpui::FontWeight::MEDIUM).text_color(t.text_faint).child(text.to_string())
}

#[allow(clippy::too_many_arguments)]
fn choice_row<V: 'static>(
    key: &str,
    field: &str,
    label: &str,
    choices: &[String],
    state: &State,
    t: &Theme,
    weak: &WeakEntity<V>,
    handler: &Handler<V>,
) -> AnyElement {
    let current = state.get(field).to_string();
    let mut chips = div().flex().flex_wrap().gap(px(6.0));
    for (i, choice) in choices.iter().enumerate() {
        let on = *choice == current;
        let (w, h) = (weak.clone(), handler.clone());
        let (fid, val) = (field.to_string(), choice.clone());
        chips = chips.child(
            div()
                .id(SharedString::from(format!("{key}-{i}")))
                .h(px(28.0))
                .px(px(11.0))
                .flex()
                .items_center()
                .rounded(px(9.0))
                .cursor_pointer()
                .role(gpui::Role::RadioButton)
                .aria_selected(on)
                .aria_label(SharedString::from(choice.clone()))
                .when(on, |d| d.bg(t.solid).text_color(t.on_solid))
                .when(!on, |d| d.bg(t.wash(0.07)).text_color(t.text).hover(|s| s.bg(t.wash(0.13))))
                .text_size(px(12.5))
                .child(choice.clone())
                .on_click(move |_, window, cx| {
                    emit(&w, &h, Event::Set { id: fid.clone(), value: val.clone() }, window, cx)
                }),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(7.0))
        .when(!label.is_empty(), |d| d.child(section_title(label, t)))
        .child(chips)
        .into_any_element()
}

/// A small trend line.
fn sparkline(values: Vec<f64>, color: Hsla, height: f32) -> impl IntoElement {
    let (lo, hi) = values.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    let span = if hi - lo < 1e-9 { 1.0 } else { hi - lo };
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let w = f32::from(bounds.size.width);
            let h = f32::from(bounds.size.height) - 4.0;
            let at = |i: usize, v: f64| {
                point(
                    bounds.origin.x + px(w * i as f32 / (values.len() - 1) as f32),
                    bounds.origin.y + px(2.0 + h * (1.0 - ((v - lo) / span) as f32)),
                )
            };
            let mut line = PathBuilder::stroke(px(1.5));
            line.move_to(at(0, values[0]));
            for (i, v) in values.iter().enumerate().skip(1) {
                line.line_to(at(i, *v));
            }
            if let Ok(path) = line.build() {
                window.paint_path(path, color);
            }
        },
    )
    .w_full()
    .h(px(height))
}

/// Series colours for charts: the accent first, then calm neighbours.
fn series_color(i: usize, t: &Theme) -> Hsla {
    let dark = t.appearance.is_dark();
    match i % 6 {
        0 => t.accent,
        1 => crate::theme::oklch(if dark { 0.72 } else { 0.55 }, 0.13, 163.0),
        2 => crate::theme::oklch(if dark { 0.80 } else { 0.62 }, 0.14, 84.0),
        3 => crate::theme::oklch(if dark { 0.70 } else { 0.55 }, 0.16, 350.0),
        4 => crate::theme::oklch(if dark { 0.70 } else { 0.52 }, 0.15, 277.0),
        _ => t.text_faint,
    }
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`.
fn parse_hex(s: &str) -> Option<Hsla> {
    let h = s.trim().trim_start_matches('#');
    let full = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 | 8 => h.to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&full[..6], 16).ok()?;
    let mut c: Hsla = gpui::rgb(v).into();
    if full.len() == 8 {
        c.a = u8::from_str_radix(&full[6..8], 16).ok()? as f32 / 255.0;
    }
    Some(c)
}

fn tone_color(tone: &str, t: &Theme) -> Hsla {
    match tone {
        "warn" | "warning" => t.warning,
        "danger" | "error" => t.danger,
        "success" | "ok" => t.success,
        _ => t.accent,
    }
}

fn render_item<V: 'static>(
    key: &str,
    item: &Item,
    state: &State,
    t: &Theme,
    weak: &WeakEntity<V>,
    handler: &Handler<V>,
) -> AnyElement {
    match item {
        Item::Text { text } => {
            div().text_size(px(13.5)).line_height(px(20.0)).text_color(t.text).child(text.clone()).into_any_element()
        }
        Item::Callout { tone, title, text } => {
            let c = tone_color(tone, t);
            div()
                .flex()
                .gap(px(10.0))
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(c.opacity(0.09))
                .border_l_2()
                .border_color(c)
                .child(
                    icon(match tone.as_str() {
                        "warn" | "warning" | "danger" | "error" => icons::DANGER_TRIANGLE,
                        "success" | "ok" => icons::CHECK,
                        _ => icons::INFO_CIRCLE,
                    })
                    .size(px(14.0))
                    .flex_none()
                    .mt(px(2.0))
                    .text_color(c),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .min_w_0()
                        .when(!title.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .child(title.clone()),
                            )
                        })
                        .child(div().text_size(px(13.0)).line_height(px(19.0)).text_color(t.text).child(text.clone())),
                )
                .into_any_element()
        }
        Item::Stats { items } => {
            let mut row = div().flex().flex_wrap().gap(px(8.0));
            for s in items {
                let up = s.delta.starts_with('+');
                let down = s.delta.starts_with('-') || s.delta.starts_with('−');
                row = row.child(
                    div()
                        .flex_1()
                        .min_w(px(110.0))
                        .p(px(12.0))
                        .rounded(px(12.0))
                        .bg(t.wash(0.045))
                        .border_1()
                        .border_color(t.border)
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(div().text_size(px(11.5)).text_color(t.text_faint).truncate().child(s.label.clone()))
                        .child(
                            div()
                                .flex()
                                .items_end()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .text_size(px(21.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(t.text)
                                        .child(s.value.clone()),
                                )
                                .when(!s.delta.is_empty(), |d| {
                                    d.child(
                                        div()
                                            .pb(px(3.0))
                                            .text_size(px(11.5))
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(if up {
                                                t.success
                                            } else if down {
                                                t.danger
                                            } else {
                                                t.text_muted
                                            })
                                            .child(s.delta.clone()),
                                    )
                                }),
                        )
                        .when(s.spark.len() > 1, |d| {
                            let color = if down {
                                t.danger
                            } else if up {
                                t.success
                            } else {
                                t.accent
                            };
                            d.child(sparkline(s.spark.clone(), color, 28.0))
                        }),
                );
            }
            row.into_any_element()
        }
        Item::Bars { title, items, unit, horizontal } => {
            let max = items.iter().map(|d| d.value).fold(0.0_f64, f64::max).max(1e-9);
            let body =
                if *horizontal {
                    let mut col = div().flex().flex_col().gap(px(7.0));
                    for (i, d) in items.iter().enumerate() {
                        let f = (d.value / max).clamp(0.0, 1.0) as f32;
                        col =
                            col.child(
                                div()
                                    .id(SharedString::from(format!("{key}-{i}")))
                                    .flex()
                                    .items_center()
                                    .gap(px(10.0))
                                    .child(
                                        div()
                                            .w(px(96.0))
                                            .flex_none()
                                            .truncate()
                                            .text_size(px(12.0))
                                            .text_color(t.text_muted)
                                            .child(d.label.clone()),
                                    )
                                    .child(div().flex_1().h(px(10.0)).rounded(px(5.0)).bg(t.wash(0.06)).child(
                                        div().h_full().w(gpui::relative(f.max(0.01))).rounded(px(5.0)).bg(t.accent),
                                    ))
                                    .child(
                                        div()
                                            .w(px(64.0))
                                            .flex_none()
                                            .text_right()
                                            .text_size(px(12.0))
                                            .text_color(t.text)
                                            .child(format!("{}{}", number(d.value), unit)),
                                    ),
                            );
                    }
                    col.into_any_element()
                } else {
                    let mut row = div().h(px(140.0)).flex().items_end().gap(px(6.0)).pt(px(16.0));
                    for (i, d) in items.iter().enumerate() {
                        let f = (d.value / max).clamp(0.0, 1.0) as f32;
                        let tip = format!("{}: {}{}", d.label, number(d.value), unit);
                        row = row.child(
                            div()
                                .id(SharedString::from(format!("{key}-{i}")))
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .flex()
                                .flex_col()
                                .justify_end()
                                .items_center()
                                .gap(px(4.0))
                                .tooltip(ui::tip(tip))
                                .child(div().text_size(px(10.5)).text_color(t.text_faint).child(number(d.value)))
                                .child(
                                    div()
                                        .w_full()
                                        .max_w(px(42.0))
                                        .h(px((96.0 * f).max(2.0)))
                                        .rounded_t(px(5.0))
                                        .bg(t.accent.opacity(0.85))
                                        .hover(|s| s.bg(t.accent)),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .truncate()
                                        .text_center()
                                        .text_size(px(11.0))
                                        .text_color(t.text_muted)
                                        .child(d.label.clone()),
                                ),
                        );
                    }
                    row.into_any_element()
                };
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .when(!title.is_empty(), |d| d.child(section_title(title, t)))
                .child(body)
                .into_any_element()
        }
        Item::Line { title, values, labels, unit } => {
            let vals = values.clone();
            let color = t.accent;
            let (lo, hi) = vals.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
            let span = if hi - lo < 1e-9 { 1.0 } else { hi - lo };
            let chart = canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    if vals.len() < 2 {
                        return;
                    }
                    let w = f32::from(bounds.size.width);
                    let h = f32::from(bounds.size.height) - 6.0;
                    let at = |i: usize, v: f64| {
                        let x = bounds.origin.x + px(w * i as f32 / (vals.len() - 1) as f32);
                        let y = bounds.origin.y + px(3.0 + h * (1.0 - ((v - lo) / span) as f32));
                        point(x, y)
                    };
                    let mut area = PathBuilder::fill();
                    area.move_to(point(bounds.origin.x, bounds.origin.y + bounds.size.height));
                    for (i, v) in vals.iter().enumerate() {
                        area.line_to(at(i, *v));
                    }
                    area.line_to(point(bounds.origin.x + bounds.size.width, bounds.origin.y + bounds.size.height));
                    area.close();
                    if let Ok(path) = area.build() {
                        window.paint_path(path, color.opacity(0.12));
                    }
                    let mut line = PathBuilder::stroke(px(2.0));
                    line.move_to(at(0, vals[0]));
                    for (i, v) in vals.iter().enumerate().skip(1) {
                        line.line_to(at(i, *v));
                    }
                    if let Ok(path) = line.build() {
                        window.paint_path(path, color);
                    }
                },
            )
            .w_full()
            .h(px(110.0));
            let first = labels.first().cloned().unwrap_or_default();
            let last = labels.last().cloned().unwrap_or_default();
            let latest = values.last().map(|v| format!("{}{}", number(*v), unit)).unwrap_or_default();
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(div().flex().items_center().child(div().flex_1().child(section_title(title, t))).child(
                    div().text_size(px(12.5)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.text).child(latest),
                ))
                .child(chart)
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_size(px(10.5))
                        .text_color(t.text_faint)
                        .child(first)
                        .child(last),
                )
                .into_any_element()
        }
        Item::Table { columns, rows } => {
            let mut table =
                div().flex().flex_col().rounded(px(10.0)).border_1().border_color(t.border).overflow_hidden();
            let ncols = columns.len().max(rows.iter().map(Vec::len).max().unwrap_or(0)).max(1);
            if !columns.is_empty() {
                let mut head = div().flex().bg(t.wash(0.05)).border_b_1().border_color(t.border);
                for c in columns {
                    head = head.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px(px(10.0))
                            .py(px(7.0))
                            .truncate()
                            .text_size(px(11.5))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text_muted)
                            .child(c.clone()),
                    );
                }
                table = table.child(head);
            }
            for (r, row) in rows.iter().take(200).enumerate() {
                let mut line = div().flex().when(r > 0, |d| d.border_t_1().border_color(t.border.opacity(0.6)));
                for c in 0..ncols {
                    line = line.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px(px(10.0))
                            .py(px(7.0))
                            .text_size(px(12.5))
                            .text_color(t.text)
                            .child(row.get(c).map(cell).unwrap_or_default()),
                    );
                }
                table = table.child(line);
            }
            table.into_any_element()
        }
        Item::Progress { label, value } => {
            let f = if *value > 1.0 { (*value / 100.0) as f32 } else { *value as f32 }.clamp(0.0, 1.0);
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .text_size(px(12.0))
                        .child(div().flex_1().text_color(t.text_muted).child(label.clone()))
                        .child(div().text_color(t.text).child(format!("{}%", (f * 100.0).round() as i32))),
                )
                .child(
                    div()
                        .h(px(8.0))
                        .rounded(px(4.0))
                        .bg(t.wash(0.07))
                        .child(div().h_full().w(gpui::relative(f.max(0.005))).rounded(px(4.0)).bg(t.accent)),
                )
                .into_any_element()
        }
        Item::Kv { items } => {
            let mut col = div().flex().flex_col().gap(px(6.0));
            for (k, v) in items {
                col = col.child(
                    div()
                        .flex()
                        .gap(px(12.0))
                        .text_size(px(12.5))
                        .child(div().w(px(132.0)).flex_none().truncate().text_color(t.text_faint).child(k.clone()))
                        .child(div().flex_1().min_w_0().text_color(t.text).child(cell(v))),
                );
            }
            col.into_any_element()
        }
        Item::Code { lang, text } => div()
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .bg(t.code_wash)
            .border_1()
            .border_color(t.border)
            .overflow_hidden()
            .when(!lang.is_empty(), |d| {
                d.child(div().px(px(10.0)).pt(px(6.0)).text_size(px(10.5)).text_color(t.text_faint).child(lang.clone()))
            })
            .child(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .font_family(t.font_mono.clone())
                    .text_size(px(12.0))
                    .line_height(px(18.0))
                    .text_color(t.code_text)
                    .child(text.clone()),
            )
            .into_any_element(),
        Item::Image { src, alt } => div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(gpui::img(SharedString::from(src.clone())).max_w_full().max_h(px(320.0)).rounded(px(10.0)))
            .when(!alt.is_empty(), |d| d.child(div().text_size(px(11.5)).text_color(t.text_faint).child(alt.clone())))
            .into_any_element(),
        Item::Checklist { id, label, items } => {
            let checked = state.checked(id);
            let mut col = div().flex().flex_col().gap(px(2.0));
            for (i, c) in items.iter().enumerate() {
                let on = checked.contains(&c.label);
                let (w, h) = (weak.clone(), handler.clone());
                let mut next = checked.clone();
                if on {
                    next.retain(|l| l != &c.label);
                } else {
                    next.push(c.label.clone());
                }
                // Keep the spec's order.
                let ordered: Vec<String> = items.iter().map(|x| x.label.clone()).filter(|l| next.contains(l)).collect();
                let fid = id.clone();
                col = col.child(
                    div()
                        .id(SharedString::from(format!("{key}-{i}")))
                        .flex()
                        .items_center()
                        .gap(px(9.0))
                        .px(px(6.0))
                        .py(px(5.0))
                        .rounded(px(7.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.wash(0.05)))
                        .role(gpui::Role::CheckBox)
                        .aria_toggled(if on { gpui::Toggled::True } else { gpui::Toggled::False })
                        .aria_label(SharedString::from(c.label.clone()))
                        .child(
                            div()
                                .size(px(16.0))
                                .flex_none()
                                .rounded(px(5.0))
                                .border_1()
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(on, |d| d.bg(t.accent).border_color(t.accent))
                                .when(!on, |d| d.border_color(t.border_strong))
                                .when(on, |d| d.child(icon(icons::CHECK).size(px(11.0)).text_color(gpui::white()))),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(if on { t.text_muted } else { t.text })
                                .child(c.label.clone()),
                        )
                        .on_click(move |_, window, cx| {
                            emit(&w, &h, Event::Set { id: fid.clone(), value: ordered.join(", ") }, window, cx)
                        }),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .when(!label.is_empty(), |d| d.child(section_title(label, t)))
                .child(col)
                .into_any_element()
        }
        Item::Toggle { id, label, .. } => {
            let on = state.get(id) == "on";
            let (w, h) = (weak.clone(), handler.clone());
            let fid = id.clone();
            div()
                .id(SharedString::from(key.to_string()))
                .flex()
                .items_center()
                .gap(px(10.0))
                .cursor_pointer()
                .role(gpui::Role::Switch)
                .aria_toggled(if on { gpui::Toggled::True } else { gpui::Toggled::False })
                .aria_label(SharedString::from(label.clone()))
                .child(div().flex_1().text_size(px(13.0)).text_color(t.text).child(label.clone()))
                .child(
                    div()
                        .w(px(34.0))
                        .h(px(20.0))
                        .rounded(px(10.0))
                        .p(px(2.0))
                        .flex()
                        .when(on, |d| d.bg(t.accent).justify_end())
                        .when(!on, |d| d.bg(t.wash(0.16)))
                        .child(div().size(px(16.0)).rounded_full().bg(gpui::white())),
                )
                .on_click(move |_, window, cx| {
                    emit(
                        &w,
                        &h,
                        Event::Set { id: fid.clone(), value: if on { "off" } else { "on" }.into() },
                        window,
                        cx,
                    )
                })
                .into_any_element()
        }
        Item::Number { id, label, min, max, step, .. } => {
            let cur: f64 = state.get(id).parse().unwrap_or(0.0);
            let step = step.unwrap_or(1.0).abs().max(1e-9);
            let clamp = |v: f64| {
                let v = min.map_or(v, |m| v.max(m));
                max.map_or(v, |m| v.min(m))
            };
            let stepper = |dir: f64, glyph: &'static str, label: &'static str| {
                let next = clamp(cur + dir * step);
                let enabled = (next - cur).abs() > 1e-12;
                let (w, h) = (weak.clone(), handler.clone());
                let fid = id.clone();
                div()
                    .id(SharedString::from(format!("{key}-{label}")))
                    .size(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.0))
                    .bg(t.wash(0.07))
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .when(enabled, |d| {
                        d.cursor_pointer().hover(|s| s.bg(t.wash(0.14))).on_click(move |_, window, cx| {
                            emit(&w, &h, Event::Set { id: fid.clone(), value: number(next) }, window, cx)
                        })
                    })
                    .when(!enabled, |d| d.opacity(0.4))
                    .child(div().text_size(px(15.0)).text_color(t.text).child(glyph))
            };
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(div().flex_1().text_size(px(13.0)).text_color(t.text).child(label.clone()))
                .child(stepper(-1.0, "−", "Decrease"))
                .child(
                    div()
                        .min_w(px(44.0))
                        .text_center()
                        .text_size(px(14.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(t.text)
                        .child(number(cur)),
                )
                .child(stepper(1.0, "+", "Increase"))
                .into_any_element()
        }
        Item::Choice { id, label, choices, .. } => choice_row(key, id, label, choices, state, t, weak, handler),
        Item::Divider => div().h(px(1.0)).bg(t.border).into_any_element(),
        Item::Timeline { title, steps } => {
            let mut col = div().flex().flex_col();
            let last = steps.len().saturating_sub(1);
            for (i, step) in steps.iter().enumerate() {
                let (color, glyph) = match step.status.as_str() {
                    "done" | "complete" | "completed" => (t.success, Some(icons::CHECK)),
                    "active" | "running" | "current" => (t.accent, None),
                    "failed" | "error" => (t.danger, Some(icons::CLOSE)),
                    _ => (t.text_faint, None),
                };
                let active = matches!(step.status.as_str(), "active" | "running" | "current");
                col = col.child(
                    div()
                        .flex()
                        .gap(px(12.0))
                        .child(
                            div()
                                .w(px(18.0))
                                .flex_none()
                                .flex()
                                .flex_col()
                                .items_center()
                                .child(
                                    div()
                                        .size(px(18.0))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .border_1()
                                        .border_color(color)
                                        .when(glyph.is_some(), |d| d.bg(color))
                                        .when(active, |d| d.child(div().size(px(8.0)).rounded_full().bg(color)))
                                        .when_some(glyph, |d, g| {
                                            d.child(icon(g).size(px(11.0)).text_color(gpui::white()))
                                        }),
                                )
                                .when(i < last, |d| d.child(div().w(px(1.5)).flex_1().min_h(px(14.0)).bg(t.border))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .pb(px(if i < last { 12.0 } else { 0.0 }))
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(if active {
                                            gpui::FontWeight::SEMIBOLD
                                        } else {
                                            gpui::FontWeight::NORMAL
                                        })
                                        .text_color(if step.status == "todo" || step.status.is_empty() {
                                            t.text_muted
                                        } else {
                                            t.text
                                        })
                                        .child(step.title.clone()),
                                )
                                .when(!step.detail.is_empty(), |d| {
                                    d.child(
                                        div().text_size(px(12.0)).text_color(t.text_faint).child(step.detail.clone()),
                                    )
                                }),
                        ),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .when(!title.is_empty(), |d| d.child(section_title(title, t)))
                .child(col)
                .into_any_element()
        }
        Item::Cards { id, label, items, .. } => {
            let current = state.get(id).to_string();
            let mut grid = div().flex().flex_wrap().gap(px(8.0));
            for (i, c) in items.iter().enumerate() {
                let on = c.title == current || c.id == current;
                let (w, h) = (weak.clone(), handler.clone());
                let (fid, val) = (id.clone(), c.title.clone());
                grid = grid.child(
                    div()
                        .id(SharedString::from(format!("{key}-{i}")))
                        .flex_1()
                        .min_w(px(150.0))
                        .p(px(12.0))
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .rounded(px(12.0))
                        .border_1()
                        .cursor_pointer()
                        .role(gpui::Role::RadioButton)
                        .aria_selected(on)
                        .aria_label(SharedString::from(c.title.clone()))
                        .when(on, |d| d.border_color(t.accent).bg(t.accent.opacity(0.08)))
                        .when(!on, |d| {
                            d.border_color(t.border).hover(|s| s.bg(t.wash(0.04)).border_color(t.border_strong))
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(13.5))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(t.text)
                                        .child(c.title.clone()),
                                )
                                .when(!c.badge.is_empty(), |d| {
                                    d.child(
                                        div()
                                            .px(px(6.0))
                                            .h(px(18.0))
                                            .flex()
                                            .items_center()
                                            .rounded(px(5.0))
                                            .bg(t.accent.opacity(0.14))
                                            .text_size(px(10.5))
                                            .text_color(t.accent)
                                            .child(c.badge.clone()),
                                    )
                                })
                                .when(on, |d| d.child(icon(icons::CHECK).size(px(15.0)).text_color(t.accent))),
                        )
                        .when(!c.body.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_size(px(12.0))
                                    .line_height(px(17.0))
                                    .text_color(t.text_muted)
                                    .child(c.body.clone()),
                            )
                        })
                        .on_click(move |_, window, cx| {
                            emit(&w, &h, Event::Set { id: fid.clone(), value: val.clone() }, window, cx)
                        }),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap(px(7.0))
                .when(!label.is_empty(), |d| d.child(section_title(label, t)))
                .child(grid)
                .into_any_element()
        }
        Item::Breakdown { title, items, unit } => {
            let total: f64 = items.iter().map(|d| d.value.max(0.0)).sum::<f64>().max(1e-9);
            let palette = [t.accent, t.success, t.warning, t.danger, t.busy, t.text_muted];
            let mut bar = div().h(px(12.0)).w_full().flex().rounded(px(6.0)).overflow_hidden().gap(px(2.0));
            let mut legend = div().flex().flex_wrap().gap_x(px(14.0)).gap_y(px(6.0));
            for (i, d) in items.iter().enumerate() {
                let c = palette[i % palette.len()];
                let f = (d.value.max(0.0) / total) as f32;
                bar = bar.child(div().h_full().w(gpui::relative(f)).bg(c));
                legend = legend.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(12.0))
                        .child(div().size(px(8.0)).rounded(px(2.0)).bg(c))
                        .child(div().text_color(t.text_muted).child(d.label.clone()))
                        .child(div().text_color(t.text).child(if unit.trim() == "%" {
                            format!("{:.0}%", f * 100.0)
                        } else {
                            format!("{}{} · {:.0}%", number(d.value), unit, f * 100.0)
                        })),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .when(!title.is_empty(), |d| d.child(section_title(title, t)))
                .child(bar)
                .child(legend)
                .into_any_element()
        }
        Item::Tabs { id, tabs } => {
            let current = state.get(id).to_string();
            let sel = tabs.iter().position(|p| p.label == current).unwrap_or(0);
            let mut strip = div().flex().gap(px(2.0)).p(px(2.0)).rounded(px(10.0)).bg(t.wash(0.05));
            for (i, pane) in tabs.iter().enumerate() {
                let on = i == sel;
                let (w, h) = (weak.clone(), handler.clone());
                let (fid, val) = (id.clone(), pane.label.clone());
                strip = strip.child(
                    div()
                        .id(SharedString::from(format!("{key}-t{i}")))
                        .h(px(26.0))
                        .px(px(12.0))
                        .flex()
                        .items_center()
                        .rounded(px(8.0))
                        .text_size(px(12.5))
                        .cursor_pointer()
                        .role(gpui::Role::Tab)
                        .aria_selected(on)
                        .when(on, |d| d.bg(t.surface_raised).text_color(t.text).shadow_sm())
                        .when(!on, |d| d.text_color(t.text_muted).hover(|s| s.text_color(t.text)))
                        .child(pane.label.clone())
                        .on_click(move |_, window, cx| {
                            emit(&w, &h, Event::Set { id: fid.clone(), value: val.clone() }, window, cx)
                        }),
                );
            }
            let mut body = div().flex().flex_col().gap(px(12.0));
            if let Some(pane) = tabs.get(sel) {
                for (n, b) in pane.blocks.iter().enumerate() {
                    body = body.child(render_item(&format!("{key}-p{sel}-{n}"), b, state, t, weak, handler));
                }
            }
            div().flex().flex_col().gap(px(12.0)).child(div().flex().child(strip)).child(body).into_any_element()
        }
        Item::Heading { text } => div()
            .pt(px(2.0))
            .text_size(px(14.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(t.text)
            .child(text.clone())
            .into_any_element(),
        Item::Donut { title, items, unit } => {
            let total: f64 = items.iter().map(|d| d.value.max(0.0)).sum::<f64>().max(1e-9);
            let colors: Vec<Hsla> = (0..items.len()).map(|i| series_color(i, t)).collect();
            let fracs: Vec<f32> = items.iter().map(|d| (d.value.max(0.0) / total) as f32).collect();
            let ring = {
                let (fracs, colors) = (fracs.clone(), colors.clone());
                let track = t.wash(0.06);
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let size = f32::from(bounds.size.width.min(bounds.size.height));
                        let c = point(
                            bounds.origin.x + px(f32::from(bounds.size.width) / 2.0),
                            bounds.origin.y + px(f32::from(bounds.size.height) / 2.0),
                        );
                        let r = size / 2.0 - 8.0;
                        let arc = |from: f32, to: f32, color: Hsla, window: &mut Window| {
                            let steps = (((to - from) * 64.0).ceil() as usize).max(2);
                            let mut b = PathBuilder::stroke(px(12.0));
                            for k in 0..=steps {
                                let a = (from + (to - from) * k as f32 / steps as f32) * std::f32::consts::TAU
                                    - std::f32::consts::FRAC_PI_2;
                                let p = point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
                                if k == 0 { b.move_to(p) } else { b.line_to(p) }
                            }
                            if let Ok(path) = b.build() {
                                window.paint_path(path, color);
                            }
                        };
                        arc(0.0, 0.9999, track, window);
                        let mut at = 0.0;
                        for (f, color) in fracs.iter().zip(&colors) {
                            if *f > 0.0005 {
                                // A hairline gap between segments.
                                let gap = if fracs.len() > 1 { 0.004 } else { 0.0 };
                                arc(at + gap, at + f - gap, *color, window);
                            }
                            at += f;
                        }
                    },
                )
                .size(px(132.0))
                .flex_none()
            };
            let legend = div().flex_1().min_w_0().flex().flex_col().gap(px(7.0)).children(
                items.iter().enumerate().map(|(i, d)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(px(12.5))
                        .child(div().size(px(9.0)).flex_none().rounded(px(3.0)).bg(colors[i]))
                        .child(div().flex_1().min_w_0().truncate().text_color(t.text_muted).child(d.label.clone()))
                        .child(div().text_color(t.text).child(format!("{}{unit}", number(d.value))))
                        .child(
                            div()
                                .w(px(40.0))
                                .text_right()
                                .text_color(t.text_faint)
                                .child(format!("{:.0}%", fracs[i] * 100.0)),
                        )
                }),
            );
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .when(!title.is_empty(), |d| d.child(section_title(title, t)))
                .child(div().flex().items_center().gap(px(18.0)).child(ring).child(legend))
                .into_any_element()
        }
        Item::Swatches { title, items } => div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .when(!title.is_empty(), |d| d.child(section_title(title, t)))
            .child(div().flex().flex_wrap().gap(px(8.0)).children(items.iter().enumerate().map(|(i, sw)| {
                let color = parse_hex(&sw.hex).unwrap_or(t.wash(0.1));
                let hex = sw.hex.clone();
                div()
                    .id(SharedString::from(format!("{key}-sw-{i}")))
                    .w(px(96.0))
                    .flex()
                    .flex_col()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(t.border)
                    .overflow_hidden()
                    .cursor_pointer()
                    .tooltip(ui::tip("Copy hex"))
                    .child(div().h(px(52.0)).bg(color))
                    .child(
                        div()
                            .px(px(8.0))
                            .py(px(6.0))
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(12.0)).text_color(t.text).truncate().child(sw.name.clone()))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_family(t.font_mono.clone())
                                    .text_color(t.text_faint)
                                    .child(sw.hex.clone()),
                            ),
                    )
                    .on_click(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(hex.clone())))
            })))
            .into_any_element(),
        Item::Gallery { items } => div()
            .flex()
            .flex_wrap()
            .gap(px(8.0))
            .children(items.iter().map(|shot| {
                div()
                    .w(px(168.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        gpui::img(SharedString::from(shot.src.clone()))
                            .w(px(168.0))
                            .h(px(112.0))
                            .rounded(px(10.0))
                            .object_fit(gpui::ObjectFit::Cover),
                    )
                    .when(!shot.caption.is_empty(), |d| {
                        d.child(
                            div().text_size(px(11.5)).text_color(t.text_faint).truncate().child(shot.caption.clone()),
                        )
                    })
            }))
            .into_any_element(),
        // Columns share the card's width and wrap to a new line before they
        // would spill past its edge.
        Item::Board { columns } => div()
            .flex()
            .flex_wrap()
            .gap(px(8.0))
            .children(columns.iter().map(|col| {
                div()
                    .flex_1()
                    .min_w(px(110.0))
                    .p(px(8.0))
                    .rounded(px(10.0))
                    .bg(t.wash(0.035))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(2.0))
                            .text_size(px(11.5))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text_muted)
                            .child(col.title.clone())
                            .child(div().text_color(t.text_faint).child(col.items.len().to_string())),
                    )
                    .children(col.items.iter().map(|item| {
                        div()
                            .px(px(9.0))
                            .py(px(7.0))
                            .rounded(px(8.0))
                            .bg(t.surface_card)
                            .border_1()
                            .border_color(t.border)
                            .text_size(px(12.5))
                            .text_color(t.text)
                            .child(item.clone())
                    }))
            }))
            .into_any_element(),
        Item::Replies { label, items } => {
            let mut row = div().flex().flex_wrap().gap(px(6.0));
            for (i, r) in items.iter().enumerate() {
                let (w, h) = (weak.clone(), handler.clone());
                let action = if r.id.is_empty() { r.label.clone() } else { r.id.clone() };
                let message = r.label.clone();
                row = row.child(
                    div()
                        .id(SharedString::from(format!("{key}-r{i}")))
                        .h(px(30.0))
                        .px(px(12.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .rounded(px(15.0))
                        .border_1()
                        .border_color(t.border_strong)
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .text_color(t.text)
                        .hover(|s| s.bg(t.wash(0.07)))
                        .role(gpui::Role::Button)
                        .child(r.label.clone())
                        .child(icon(icons::ARROW_UP_RIGHT).size(px(11.0)).text_color(t.text_faint))
                        .on_click(move |_, window, cx| {
                            emit(&w, &h, Event::Act { action: action.clone(), message: message.clone() }, window, cx)
                        }),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap(px(7.0))
                .when(!label.is_empty(), |d| d.child(section_title(label, t)))
                .child(row)
                .into_any_element()
        }
        Item::Slider { id, label, min, max, step, unit, .. } => {
            let (lo, hi) = (*min, if *max > *min { *max } else { *min + 1.0 });
            let current: f64 = state.get(id).parse().unwrap_or(lo);
            let f = ((current - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
            let step = step.unwrap_or(((hi - lo) / 100.0).max(1e-9));
            let bounds = Rc::new(std::cell::Cell::new(None::<gpui::Bounds<gpui::Pixels>>));
            let probe = bounds.clone();
            let (w, h) = (weak.clone(), handler.clone());
            let fid = id.clone();
            let pick = move |x: gpui::Pixels| {
                let b = probe.get()?;
                let t = (f32::from(x - b.origin.x) / f32::from(b.size.width)).clamp(0.0, 1.0) as f64;
                let raw = lo + t * (hi - lo);
                Some(((raw - lo) / step).round() * step + lo)
            };
            let pick_down = pick.clone();
            let (w2, h2, fid2) = (w.clone(), h.clone(), fid.clone());
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .text_size(px(12.5))
                        .child(div().flex_1().text_color(t.text_muted).child(label.clone()))
                        .child(div().text_color(t.text).child(format!("{}{unit}", number(current)))),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("{key}-track")))
                        .relative()
                        .h(px(20.0))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .child(canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {}).absolute().inset_0())
                        .child(
                            div()
                                .w_full()
                                .h(px(6.0))
                                .rounded(px(3.0))
                                .bg(t.wash(0.09))
                                .child(div().h_full().w(gpui::relative(f.max(0.002))).rounded(px(3.0)).bg(t.accent)),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(gpui::relative(f))
                                .ml(px(-8.0))
                                .size(px(16.0))
                                .rounded_full()
                                .bg(gpui::white())
                                .border_1()
                                .border_color(t.border_strong)
                                .shadow_sm(),
                        )
                        .on_mouse_down(gpui::MouseButton::Left, move |e, window, cx| {
                            if let Some(v) = pick_down(e.position.x) {
                                emit(&w2, &h2, Event::Set { id: fid2.clone(), value: number(v) }, window, cx);
                            }
                        })
                        .on_mouse_move(move |e, window, cx| {
                            if e.pressed_button == Some(gpui::MouseButton::Left)
                                && let Some(v) = pick(e.position.x)
                            {
                                emit(&w, &h, Event::Set { id: fid.clone(), value: number(v) }, window, cx);
                            }
                        }),
                )
                .into_any_element()
        }
        Item::Rating { id, label, max, .. } => {
            let current: u32 = state.get(id).parse().unwrap_or(0);
            let mut stars = div().flex().gap(px(4.0));
            for n in 1..=(*max).clamp(1, 10) {
                let on = n <= current;
                let (w, h) = (weak.clone(), handler.clone());
                let fid = id.clone();
                stars = stars.child(
                    div()
                        .id(SharedString::from(format!("{key}-s{n}")))
                        .cursor_pointer()
                        .child(icon(if on { icons::STAR_BOLD } else { icons::STAR }).size(px(20.0)).text_color(if on {
                            t.warning
                        } else {
                            t.text_faint
                        }))
                        .on_click(move |_, window, cx| {
                            emit(&w, &h, Event::Set { id: fid.clone(), value: n.to_string() }, window, cx)
                        }),
                );
            }
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(div().flex_1().text_size(px(13.0)).text_color(t.text).child(label.clone()))
                .child(stars)
                .into_any_element()
        }
        Item::Link { label, url } => {
            let url = url.clone();
            div()
                .id(SharedString::from(format!("{key}-link")))
                .flex()
                .items_center()
                .gap(px(6.0))
                .self_start()
                .text_size(px(13.0))
                .text_color(t.accent)
                .cursor_pointer()
                .hover(|s| s.underline())
                .child(label.clone())
                .child(icon(icons::ARROW_UP_RIGHT).size(px(12.0)).text_color(t.accent))
                .on_click(move |_, _, cx| cx.open_url(&url))
                .into_any_element()
        }
        Item::Mockup { device, title, nav, blocks } => {
            let browser =
                device == "browser" || device == "web" || device == "desktop" || device == "app" || device == "window";
            let tablet = device == "tablet" || device == "ipad";
            let nav_id = format!("{key}-nav");
            let current_nav = state.get(&nav_id).to_string();
            let active_nav =
                if current_nav.is_empty() { nav.first().cloned().unwrap_or_default() } else { current_nav };
            let inner = div().flex().flex_col().gap(px(12.0)).p(px(16.0)).children(
                blocks.iter().enumerate().map(|(n, b)| render_item(&format!("{key}-m{n}"), b, state, t, weak, handler)),
            );
            let screen = div().bg(t.bg).overflow_hidden().child(inner);
            if browser {
                div()
                    .w_full()
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(t.border_strong)
                    .overflow_hidden()
                    .shadow_md()
                    .child(
                        div()
                            .h(px(32.0))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .bg(t.wash(0.05))
                            .border_b_1()
                            .border_color(t.border)
                            .child(
                                div().flex().gap(px(5.0)).children(
                                    [0xff5f57u32, 0xfebc2e, 0x28c840].map(|c| ui::dot(gpui::rgb(c).into(), 9.0)),
                                ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(20.0))
                                    .px(px(10.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(6.0))
                                    .bg(t.wash(0.06))
                                    .text_size(px(11.0))
                                    .text_color(t.text_faint)
                                    .truncate()
                                    .child(if title.is_empty() {
                                        "https://app.local/preview".to_string()
                                    } else {
                                        title.clone()
                                    }),
                            )
                            .when(!nav.is_empty(), |d| {
                                d.child(div().flex().items_center().gap(px(4.0)).children(nav.iter().enumerate().map(
                                    |(ni, item)| {
                                        let on = *item == active_nav;
                                        let (w, h) = (weak.clone(), handler.clone());
                                        let nid = nav_id.clone();
                                        let val = item.clone();
                                        div()
                                            .id(SharedString::from(format!("{key}-bnav-{ni}")))
                                            .h(px(20.0))
                                            .px(px(7.0))
                                            .flex()
                                            .items_center()
                                            .rounded(px(5.0))
                                            .cursor_pointer()
                                            .text_size(px(11.0))
                                            .when(on, |s| s.bg(t.surface_raised).text_color(t.text))
                                            .when(!on, |s| s.text_color(t.text_muted).hover(|h| h.text_color(t.text)))
                                            .child(item.clone())
                                            .on_click(move |_, window, cx| {
                                                emit(
                                                    &w,
                                                    &h,
                                                    Event::Set { id: nid.clone(), value: val.clone() },
                                                    window,
                                                    cx,
                                                )
                                            })
                                    },
                                )))
                            }),
                    )
                    .child(screen)
                    .into_any_element()
            } else {
                let frame_w = if tablet { 480.0 } else { 310.0 };
                div()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w(px(frame_w))
                            .p(px(9.0))
                            .rounded(px(if tablet { 28.0 } else { 40.0 }))
                            .bg(gpui::hsla(0.0, 0.0, 0.07, 1.0))
                            .border_1()
                            .border_color(gpui::hsla(0.0, 0.0, 1.0, 0.12))
                            .shadow_lg()
                            .child(
                                div()
                                    .rounded(px(if tablet { 20.0 } else { 32.0 }))
                                    .overflow_hidden()
                                    .bg(t.bg)
                                    .child(
                                        div()
                                            .h(px(30.0))
                                            .px(px(20.0))
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .text_size(px(11.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(t.text)
                                            .child("9:41")
                                            .when(!tablet, |d| {
                                                d.child(
                                                    div().w(px(70.0)).h(px(18.0)).rounded(px(9.0)).bg(gpui::black()),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .w(px(22.0))
                                                    .h(px(10.0))
                                                    .rounded(px(3.0))
                                                    .border_1()
                                                    .border_color(t.text_muted),
                                            ),
                                    )
                                    .when(!title.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .px(px(16.0))
                                                .pt(px(4.0))
                                                .text_size(px(18.0))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(t.text)
                                                .child(title.clone()),
                                        )
                                    })
                                    .child(screen.min_h(px(if tablet { 340.0 } else { 400.0 })))
                                    .when(!nav.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .h(px(44.0))
                                                .px(px(8.0))
                                                .flex()
                                                .items_center()
                                                .justify_around()
                                                .border_t_1()
                                                .border_color(t.border)
                                                .bg(t.wash(0.03))
                                                .children(nav.iter().enumerate().map(|(ni, item)| {
                                                    let on = *item == active_nav;
                                                    let (w, h) = (weak.clone(), handler.clone());
                                                    let nid = nav_id.clone();
                                                    let val = item.clone();
                                                    div()
                                                        .id(SharedString::from(format!("{key}-mnav-{ni}")))
                                                        .px(px(8.0))
                                                        .py(px(4.0))
                                                        .rounded(px(6.0))
                                                        .cursor_pointer()
                                                        .text_size(px(11.0))
                                                        .font_weight(if on {
                                                            gpui::FontWeight::SEMIBOLD
                                                        } else {
                                                            gpui::FontWeight::NORMAL
                                                        })
                                                        .text_color(if on { t.accent } else { t.text_faint })
                                                        .child(item.clone())
                                                        .on_click(move |_, window, cx| {
                                                            emit(
                                                                &w,
                                                                &h,
                                                                Event::Set { id: nid.clone(), value: val.clone() },
                                                                window,
                                                                cx,
                                                            )
                                                        })
                                                })),
                                        )
                                    })
                                    .child(div().h(px(20.0)).flex().items_center().justify_center().child(
                                        div().w(px(108.0)).h(px(4.0)).rounded(px(2.0)).bg(t.text.opacity(0.35)),
                                    )),
                            ),
                    )
                    .into_any_element()
            }
        }
        Item::Hero { badge, title, subtitle, cta, secondary } => div()
            .p(px(20.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(t.border)
            .bg(t.wash(0.03))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .when(!badge.is_empty(), |d| {
                d.child(
                    div()
                        .self_start()
                        .px(px(8.0))
                        .py(px(2.0))
                        .rounded(px(99.0))
                        .bg(t.accent.opacity(0.14))
                        .text_size(px(11.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.accent)
                        .child(badge.clone()),
                )
            })
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(title.clone()),
            )
            .when(!subtitle.is_empty(), |d| {
                d.child(div().text_size(px(13.5)).text_color(t.text_muted).child(subtitle.clone()))
            })
            .when(!cta.is_empty() || !secondary.is_empty(), |d| {
                let (w1, h1) = (weak.clone(), handler.clone());
                let (w2, h2) = (weak.clone(), handler.clone());
                let cta_label = cta.clone();
                let sec_label = secondary.clone();
                d.child(
                    div()
                        .pt(px(4.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .when(!cta.is_empty(), |r| {
                            r.child(
                                ui::button_primary(SharedString::from(format!("{key}-hero-cta")), cta.clone(), t)
                                    .on_click(move |_, window, cx| {
                                        emit(
                                            &w1,
                                            &h1,
                                            Event::Act { action: cta_label.clone(), message: cta_label.clone() },
                                            window,
                                            cx,
                                        )
                                    }),
                            )
                        })
                        .when(!secondary.is_empty(), |r| {
                            r.child(
                                ui::button_ghost(SharedString::from(format!("{key}-hero-sec")), secondary.clone(), t)
                                    .on_click(move |_, window, cx| {
                                        emit(
                                            &w2,
                                            &h2,
                                            Event::Act { action: sec_label.clone(), message: sec_label.clone() },
                                            window,
                                            cx,
                                        )
                                    }),
                            )
                        }),
                )
            })
            .into_any_element(),
        Item::Navbar { id, brand, items, cta } => {
            let nav_id = if id.is_empty() { format!("{key}-navbar") } else { id.clone() };
            let cur = state.get(&nav_id);
            let active = if cur.is_empty() { items.first().map(|s| s.as_str()).unwrap_or("") } else { cur };
            let (w_cta, h_cta) = (weak.clone(), handler.clone());
            let cta_txt = cta.clone();
            div()
                .h(px(40.0))
                .px(px(12.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(t.border)
                .bg(t.wash(0.03))
                .flex()
                .items_center()
                .gap(px(12.0))
                .when(!brand.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(brand.clone()),
                    )
                })
                .child(div().flex_1().flex().items_center().gap(px(4.0)).children(items.iter().enumerate().map(
                    |(ni, item)| {
                        let on = item == active;
                        let (w, h) = (weak.clone(), handler.clone());
                        let nid = nav_id.clone();
                        let val = item.clone();
                        div()
                            .id(SharedString::from(format!("{key}-nav-{ni}")))
                            .h(px(26.0))
                            .px(px(9.0))
                            .flex()
                            .items_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .text_size(px(12.0))
                            .when(on, |s| {
                                s.bg(t.surface_raised).text_color(t.text).font_weight(gpui::FontWeight::MEDIUM)
                            })
                            .when(!on, |s| s.text_color(t.text_muted).hover(|h| h.text_color(t.text)))
                            .child(item.clone())
                            .on_click(move |_, window, cx| {
                                emit(&w, &h, Event::Set { id: nid.clone(), value: val.clone() }, window, cx)
                            })
                    },
                )))
                .when(!cta.is_empty(), |d| {
                    d.child(
                        ui::button_primary(SharedString::from(format!("{key}-nav-cta")), cta.clone(), t)
                            .h(px(26.0))
                            .text_size(px(11.5))
                            .on_click(move |_, window, cx| {
                                emit(
                                    &w_cta,
                                    &h_cta,
                                    Event::Act { action: cta_txt.clone(), message: cta_txt.clone() },
                                    window,
                                    cx,
                                )
                            }),
                    )
                })
                .into_any_element()
        }
        Item::Pricing { id, plans } => {
            let pid = if id.is_empty() { format!("{key}-pricing") } else { id.clone() };
            let cur = state.get(&pid);
            let active = if cur.is_empty() { plans.first().map(|p| p.name.as_str()).unwrap_or("") } else { cur };
            div()
                .flex()
                .gap(px(10.0))
                .children(plans.iter().enumerate().map(|(pi, plan)| {
                    let selected = plan.name == active || (!plan.id.is_empty() && plan.id == active);
                    let (w, h) = (weak.clone(), handler.clone());
                    let pid = pid.clone();
                    let val = plan.name.clone();
                    div()
                        .id(SharedString::from(format!("{key}-plan-{pi}")))
                        .flex_1()
                        .min_w(px(150.0))
                        .p(px(14.0))
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(if selected { t.accent } else { t.border })
                        .bg(if selected { t.accent.opacity(0.06) } else { t.wash(0.025) })
                        .cursor_pointer()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(13.5))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(t.text)
                                        .child(plan.name.clone()),
                                )
                                .when(!plan.badge.is_empty(), |d| {
                                    d.child(
                                        div()
                                            .px(px(6.0))
                                            .py(px(1.0))
                                            .rounded(px(99.0))
                                            .bg(t.accent.opacity(0.16))
                                            .text_size(px(10.5))
                                            .text_color(t.accent)
                                            .child(plan.badge.clone()),
                                    )
                                }),
                        )
                        .when(!plan.price.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_size(px(20.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .child(plan.price.clone()),
                            )
                        })
                        .children(plan.features.iter().map(|feat| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(px(12.0))
                                .text_color(t.text_muted)
                                .child(icon(icons::CHECK).size(px(11.5)).text_color(t.success))
                                .child(feat.clone())
                        }))
                        .on_click(move |_, window, cx| {
                            emit(&w, &h, Event::Set { id: pid.clone(), value: val.clone() }, window, cx)
                        })
                }))
                .into_any_element()
        }
        Item::Split { left, right } => div()
            .flex()
            .gap(px(12.0))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(10.0)).children(
                left.iter().enumerate().map(|(n, b)| render_item(&format!("{key}-l{n}"), b, state, t, weak, handler)),
            ))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(10.0)).children(
                right.iter().enumerate().map(|(n, b)| render_item(&format!("{key}-r{n}"), b, state, t, weak, handler)),
            ))
            .into_any_element(),
        Item::Unknown => div().into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_design_blocks() {
        let spec = Spec::parse(
            r##"{"title":"Kit","blocks":[
              {"type":"donut","items":[{"label":"A","value":3},{"label":"B","value":1}]},
              {"type":"swatches","items":[{"name":"Ink","hex":"#111"}]},
              {"type":"slider","id":"size","label":"Size","value":40},
              {"type":"rating","id":"stars","label":"Rate it","value":4},
              {"type":"replies","items":[{"label":"Ship it"}]},
              {"type":"mockup","device":"phone","blocks":[{"type":"heading","text":"Home"}]},
              {"type":"stats","items":[{"label":"MRR","value":"$9k","spark":[1,2,3]}]}
            ]}"##,
        )
        .unwrap();
        assert!(matches!(spec.blocks[0], Item::Donut { .. }));
        assert!(matches!(spec.blocks[5], Item::Mockup { .. }));
        let state = State::new(spec);
        assert_eq!(state.get("size"), "40");
        assert_eq!(state.get("stars"), "4");
        assert_eq!(parse_hex("#111").map(|c| c.l < 0.1), Some(true));
    }

    #[test]
    fn parses_rich_spec_and_builds_reply() {
        let spec = Spec::parse(
            r#"{"title":"Deploy","blocks":[
                {"type":"stats","items":[{"label":"Users","value":1204,"delta":"+12%"}]},
                {"type":"bars","items":[{"label":"Mon","value":3}]},
                {"type":"toggle","id":"dry","label":"Dry run","value":true},
                {"type":"number","id":"n","label":"Replicas","value":3,"min":1,"max":5},
                {"type":"checklist","id":"c","items":[{"label":"a","checked":true},{"label":"b"}]},
                {"type":"sparkle"}
            ],"actions":[{"id":"go","label":"Ship it"}]}"#,
        )
        .unwrap();
        assert_eq!(spec.blocks.len(), 6);
        assert!(matches!(spec.blocks[5], Item::Unknown));
        let mut st = State::new(spec.clone());
        assert_eq!(st.get("dry"), "on");
        assert_eq!(st.get("n"), "3");
        assert_eq!(st.get("c"), "a");
        st.set("n", "4");
        let reply = spec.reply(&spec.actions[0], &st.values);
        assert!(reply.contains("Ship it"));
        assert!(reply.contains("Replicas: 4"));
        assert!(reply.contains("Dry run: on"));
    }

    #[test]
    fn new_blocks_parse_and_nest() {
        let spec = Spec::parse(
            r#"{"id":"plan","title":"Pick","blocks":[
                {"type":"timeline","steps":[{"title":"a","status":"done"},{"title":"b","status":"active"}]},
                {"type":"breakdown","items":[{"label":"x","value":2},{"label":"y","value":3}]},
                {"type":"cards","id":"opt","items":[{"id":"fast","title":"Fast"},{"id":"safe","title":"Safe","badge":"recommended"}]},
                {"type":"tabs","id":"view","tabs":[{"label":"One","blocks":[{"type":"toggle","id":"t","label":"T","value":true}]},{"label":"Two"}]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(spec.id, "plan");
        let st = State::new(spec);
        assert_eq!(st.get("opt"), "Fast");
        assert_eq!(st.get("view"), "One");
        assert_eq!(st.get("t"), "on");
    }

    #[test]
    fn choice_defaults_to_first() {
        let spec = Spec::parse(r#"{"title":"t","blocks":[{"type":"choice","id":"env","label":"Deploy to","choices":["staging","production"]}]}"#).unwrap();
        assert_eq!(State::new(spec).get("env"), "staging");
    }

    #[test]
    fn v1_specs_still_parse() {
        let spec = Spec::parse(
            r#"{"title":"Pick","fields":[{"id":"x","choices":["a","b"]}],"actions":[{"id":"ok","label":"OK"}]}"#,
        )
        .unwrap();
        assert_eq!(State::new(spec).get("x"), "a");
        assert!(Spec::parse("{}").is_none());
    }
}

impl Spec {
    /// A spec from a ` ```ui ` fence that is still streaming: the JSON is
    /// closed where it was cut and items that are not whole yet are left
    /// out, so the card fills in as it arrives.
    pub fn parse_partial(src: &str) -> Option<Self> {
        let mut v = repair_json(src.trim())?;
        let obj = v.as_object_mut()?;
        let mut list = |key: &str| match obj.remove(key) {
            Some(Value::Array(a)) => a,
            _ => Vec::new(),
        };
        let (blocks, actions, fields) = (list("blocks"), list("actions"), list("fields"));
        let mut spec: Spec = serde_json::from_value(v).ok()?;
        // A lone `type` may be cut mid-word ("stat" of "stats").
        spec.blocks = blocks
            .into_iter()
            .filter(|b| b.as_object().is_some_and(|o| o.len() > 1))
            .filter_map(lenient_item)
            .collect();
        spec.actions = actions.into_iter().filter_map(|b| serde_json::from_value(b).ok()).collect();
        spec.fields = fields.into_iter().filter_map(|b| serde_json::from_value(b).ok()).collect();
        Some(spec)
    }
}

/// A streaming block; if a nested list's last entry is still arriving, the
/// block shows without it.
fn lenient_item(b: Value) -> Option<Item> {
    if let Ok(item) = serde_json::from_value(b.clone()) {
        return Some(item);
    }
    let mut b = b;
    for v in b.as_object_mut()?.values_mut() {
        if let Value::Array(a) = v {
            a.pop();
        }
    }
    serde_json::from_value(b).ok()
}

/// Parse JSON that may be cut off mid-way: close an open string and every
/// open bracket; failing that, back up to the last whole member.
fn repair_json(src: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str(src) {
        return Some(v);
    }
    let (mut stack, mut cuts) = (Vec::<u8>::new(), Vec::<(usize, Vec<u8>)>::new());
    let (mut in_str, mut esc) = (false, false);
    for (i, &b) in src.as_bytes().iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' | b'[' => {
                stack.push(if b == b'{' { b'}' } else { b']' });
                cuts.push((i + 1, stack.clone()));
            }
            b'}' | b']' => {
                stack.pop();
                cuts.push((i + 1, stack.clone()));
            }
            b',' => cuts.push((i, stack.clone())),
            _ => {}
        }
    }
    if stack.is_empty() && !in_str {
        return None;
    }
    let close = |s: &str, open: &[u8]| {
        let mut out = s.trim_end().trim_end_matches(',').to_string();
        out.extend(open.iter().rev().map(|&c| c as char));
        serde_json::from_str::<Value>(&out).ok()
    };
    let mut whole = src.to_string();
    if in_str {
        if esc {
            whole.pop();
        }
        whole.push('"');
    }
    close(&whole, &stack).or_else(|| cuts.iter().rev().take(64).find_map(|(at, open)| close(&src[..*at], open)))
}

#[cfg(test)]
mod partial_tests {
    use super::*;

    #[test]
    fn partial_cards_fill_in() {
        let full = r#"{"title":"Deploy","body":"Pick a target","blocks":[{"type":"stats","items":[{"label":"Builds","value":12}]},{"type":"text","text":"Ready"}],"actions":[{"label":"Ship"}]}"#;
        let mut last_blocks = 0;
        for cut in 1..full.len() {
            if let Some(s) = Spec::parse_partial(&full[..cut]) {
                assert!(s.blocks.len() >= last_blocks, "blocks never disappear at {:?}", &full[..cut]);
                last_blocks = s.blocks.len();
            }
        }
        let s = Spec::parse_partial(&full[..full.find("Pick").unwrap() + 2]).unwrap();
        assert_eq!(s.title, "Deploy");
        assert_eq!(s.body, "Pi");
        let s = Spec::parse_partial(&full[..full.find(r#"{"type":"text""#).unwrap()]).unwrap();
        assert_eq!(s.blocks.len(), 1);
        assert!(s.actions.is_empty());
        assert_eq!(Spec::parse_partial(full).unwrap().actions.len(), 1);
    }
}
