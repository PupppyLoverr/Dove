//! Generative UI: interactive cards an agent shows inline in the chat.
//!
//! An agent emits a small JSON spec, either as a ` ```ui ` fence in its
//! reply or through the `show_ui` tool of Dove's MCP server. The card can
//! hold text, callouts, stat tiles, bar and line charts, tables, progress,
//! key/value lists, code and images, plus controls the user can change
//! (choices, toggles, number steppers, checklists). Pressing an action sends
//! the card's values back as an ordinary message, so any harness can keep
//! going; "Ask to change" puts a request to alter the card in the composer.

use std::rc::Rc;

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
}

#[derive(Debug, Clone, Deserialize)]
pub struct Datum {
    pub label: String,
    pub value: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Check {
    pub label: String,
    #[serde(default)]
    pub checked: bool,
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
    #[serde(other)]
    Unknown,
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
                    if i == id && !label.is_empty() =>
                {
                    return label.clone();
                }
                _ => {}
            }
        }
        self.fields.iter().find(|f| f.id == id && !f.label.is_empty()).map_or_else(|| id.to_string(), |f| f.label.clone())
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
        let mut values: Vec<(String, String)> = spec
            .fields
            .iter()
            .filter_map(|f| f.choices.first().cloned().map(|c| (f.id.clone(), c)))
            .collect();
        for b in &spec.blocks {
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
                _ => {}
            }
        }
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
                    .bg(t.accent.opacity(0.14))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(icons::WIDGET).size(px(13.0)).text_color(t.accent)),
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
        card = card.child(div().text_size(px(13.5)).line_height(px(20.0)).text_color(t.text_muted).child(spec.body.clone()));
    }

    for (n, item) in spec.blocks.iter().enumerate() {
        let key = format!("{id}-b{n}");
        card = card.child(render_item(&key, item, state, t, &weak, &handler));
    }

    // v1 fields.
    for field in &spec.fields {
        let label = if field.label.is_empty() { field.id.clone() } else { field.label.clone() };
        card = card.child(choice_row(&format!("{id}-f-{}", field.id), &field.id, &label, &field.choices, state, t, &weak, &handler));
    }

    if !spec.actions.is_empty() {
        let mut row = div().flex().flex_wrap().items_center().gap(px(8.0)).pt(px(2.0));
        for (i, action) in spec.actions.iter().enumerate() {
            let aid = if action.id.is_empty() { action.label.clone() } else { action.id.clone() };
            let message = spec.reply(action, &state.values);
            let el_id = SharedString::from(format!("{id}-a-{i}"));
            let btn = if i == 0 { ui::button_primary(el_id, action.label.clone(), t) } else { ui::button_ghost(el_id, action.label.clone(), t) };
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
                .on_click(move |_, window, cx| emit(&w, &h, Event::Set { id: fid.clone(), value: val.clone() }, window, cx)),
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
                            d.child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.text).child(title.clone()))
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
                        ),
                );
            }
            row.into_any_element()
        }
        Item::Bars { title, items, unit, horizontal } => {
            let max = items.iter().map(|d| d.value).fold(0.0_f64, f64::max).max(1e-9);
            let body = if *horizontal {
                let mut col = div().flex().flex_col().gap(px(7.0));
                for (i, d) in items.iter().enumerate() {
                    let f = (d.value / max).clamp(0.0, 1.0) as f32;
                    col = col.child(
                        div()
                            .id(SharedString::from(format!("{key}-{i}")))
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(div().w(px(96.0)).flex_none().truncate().text_size(px(12.0)).text_color(t.text_muted).child(d.label.clone()))
                            .child(
                                div().flex_1().h(px(10.0)).rounded(px(5.0)).bg(t.wash(0.06)).child(
                                    div().h_full().w(gpui::relative(f.max(0.01))).rounded(px(5.0)).bg(t.accent),
                                ),
                            )
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
                            .child(div().w_full().truncate().text_center().text_size(px(11.0)).text_color(t.text_muted).child(d.label.clone())),
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
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(div().flex_1().child(section_title(title, t)))
                        .child(div().text_size(px(12.5)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.text).child(latest)),
                )
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
            let mut table = div().flex().flex_col().rounded(px(10.0)).border_1().border_color(t.border).overflow_hidden();
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
            .child(
                gpui::img(SharedString::from(src.clone())).max_w_full().max_h(px(320.0)).rounded(px(10.0)),
            )
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
                    emit(&w, &h, Event::Set { id: fid.clone(), value: if on { "off" } else { "on" }.into() }, window, cx)
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
        Item::Unknown => div().into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn choice_defaults_to_first() {
        let spec = Spec::parse(r#"{"title":"t","blocks":[{"type":"choice","id":"env","label":"Deploy to","choices":["staging","production"]}]}"#).unwrap();
        assert_eq!(State::new(spec).get("env"), "staging");
    }

    #[test]
    fn v1_specs_still_parse() {
        let spec = Spec::parse(r#"{"title":"Pick","fields":[{"id":"x","choices":["a","b"]}],"actions":[{"id":"ok","label":"OK"}]}"#).unwrap();
        assert_eq!(State::new(spec).get("x"), "a");
        assert!(Spec::parse("{}").is_none());
    }
}
