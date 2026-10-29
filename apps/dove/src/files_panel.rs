//! The side panel's Files tab: the worktree as a tree with search, and an
//! inline syntax-highlighted preview (`path:line` links in the chat land
//! here) with "Open in editor".

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, StyledText, Subscription, TextRun, UniformListScrollHandle, Window, div, px, uniform_list,
};
use gpui_base::input::{Input, InputEvent, InputState};
use zeron_syntax::{HighlightRequest, HighlightedDocument};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
use crate::kit;
use crate::markdown;
use crate::markdown_extra;
use crate::office::{self, view::OfficeView, view::Target};
use crate::theme::Theme;
use crate::ui;

pub enum FilesEvent {
    AskAgent(String),
}

fn is_image(path: &str) -> bool {
    matches!(
        path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "tif" | "tiff"
    )
}

fn is_markdown(path: &str) -> bool {
    matches!(path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str(), "md" | "markdown" | "mdx")
}

/// Files bigger than this preview as plain text.
const HIGHLIGHT_MAX: usize = 512 * 1024;
const PREVIEW_MAX: usize = 4 * 1024 * 1024;
const OFFICE_MAX: u64 = 96 * 1024 * 1024;

#[derive(Clone)]
struct Entry {
    depth: usize,
    name: SharedString,
    path: String,
    dir: bool,
}

enum PreviewKind {
    Text,
    Markdown,
    Image,
    Office(office::Kind),
}

struct Preview {
    path: String,
    lines: Arc<Vec<String>>,
    highlight: Option<Arc<HighlightedDocument>>,
    markdown: Option<Arc<Vec<crate::markdown::Block>>>,
    line: Option<u32>,
    error: Option<String>,
    kind: PreviewKind,
    editing: bool,
    office: Option<OfficeView>,
}

impl EventEmitter<FilesEvent> for FilesPanel {}

pub struct FilesPanel {
    root: Option<PathBuf>,
    files: Arc<Vec<String>>,
    /// Git status per path (and a dot for folders holding changes).
    git_marks: Arc<HashMap<String, char>>,
    collapsed: HashSet<String>,
    search: Entity<InputState>,
    query: String,
    visible: Arc<Vec<Entry>>,
    preview: Option<Preview>,
    preview_scroll: UniformListScrollHandle,
    editor: Option<Entity<InputState>>,
    /// Office documents: the selected element's text, and an instruction
    /// for the agent.
    office_input: Option<Entity<InputState>>,
    agent_input: Option<Entity<InputState>>,
    office_status: Option<String>,
    office_width: std::rc::Rc<std::cell::Cell<f32>>,
    _office_subs: Vec<Subscription>,
    _load: Option<gpui::Task<()>>,
    _sub: Subscription,
}

impl FilesPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let search = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Search files");
            s.set_editor_style(style);
            s
        });
        let _sub = cx.subscribe(&search, |this, s, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.query = s.read(cx).value().to_string();
                this.rebuild();
                cx.notify();
            }
        });
        Self {
            root: None,
            files: Arc::default(),
            git_marks: Arc::default(),
            collapsed: HashSet::new(),
            search,
            query: String::new(),
            visible: Arc::default(),
            preview: None,
            preview_scroll: UniformListScrollHandle::new(),
            editor: None,
            office_input: None,
            agent_input: None,
            office_status: None,
            office_width: Default::default(),
            _office_subs: Vec::new(),
            _load: None,
            _sub,
        }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        if root == self.root {
            return;
        }
        self.root = root;
        self.collapsed.clear();
        self.preview = None;
        self.refresh(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            self.files = Arc::default();
            self.rebuild();
            cx.notify();
            return;
        };
        let work = cx.background_spawn(async move {
            let files = dove_core::git::list_files(&root).unwrap_or_default();
            let status = dove_core::git::status(&root).unwrap_or_default();
            (files, status)
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let (files, status) = work.await;
            let _ = this.update(cx, |p, cx| {
                // What changed, by path; folders show a dot when anything
                // inside them did.
                let mut marks: HashMap<String, char> = HashMap::new();
                for (path, mark) in &status {
                    marks.insert(path.trim_end_matches('/').to_string(), *mark);
                    let parts: Vec<&str> = path.split('/').collect();
                    for k in 1..parts.len() {
                        marks.entry(parts[..k].join("/")).or_insert('•');
                    }
                }
                p.git_marks = Arc::new(marks);
                // Directories start collapsed below the top level.
                let mut dirs = HashSet::new();
                for f in &files {
                    let parts: Vec<&str> = f.split('/').collect();
                    for k in 1..parts.len() {
                        dirs.insert(parts[..k].join("/"));
                    }
                }
                p.collapsed = dirs;
                p.files = Arc::new(files);
                p.rebuild();
                cx.notify();
            });
        }));
    }

    fn rebuild(&mut self) {
        let q = self.query.trim().to_lowercase();
        let mut out = Vec::new();
        if !q.is_empty() {
            for f in self.files.iter().filter(|f| f.to_lowercase().contains(&q)).take(500) {
                out.push(Entry { depth: 0, name: f.clone().into(), path: f.clone(), dir: false });
            }
        } else {
            #[derive(Default)]
            struct Node {
                dirs: BTreeMap<String, Node>,
                files: Vec<String>,
            }
            let mut root = Node::default();
            for f in self.files.iter() {
                let mut node = &mut root;
                let mut parts = f.split('/').peekable();
                while let Some(p) = parts.next() {
                    if parts.peek().is_some() {
                        node = node.dirs.entry(p.to_string()).or_default();
                    } else {
                        node.files.push(p.to_string());
                    }
                }
            }
            fn walk(n: &Node, prefix: &str, depth: usize, collapsed: &HashSet<String>, out: &mut Vec<Entry>) {
                for (name, child) in &n.dirs {
                    let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                    out.push(Entry { depth, name: name.clone().into(), path: path.clone(), dir: true });
                    if !collapsed.contains(&path) {
                        walk(child, &path, depth + 1, collapsed, out);
                    }
                }
                for name in &n.files {
                    let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                    out.push(Entry { depth, name: name.clone().into(), path, dir: false });
                }
            }
            walk(&root, "", 0, &self.collapsed, &mut out);
        }
        self.visible = Arc::new(out);
    }

    fn toggle_dir(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_string());
        }
        self.rebuild();
        cx.notify();
    }

    /// Preview `path` (relative to the root, or absolute), scrolled to `line`.
    pub fn open_path(&mut self, path: &str, line: Option<u32>, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let full = {
            let p = PathBuf::from(path);
            if p.is_absolute() { p } else { root.join(path.trim_start_matches("./")) }
        };
        let rel = full
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string());
        let office_kind = office::Kind::of(&rel);
        let kind = if let Some(k) = office_kind {
            PreviewKind::Office(k)
        } else if is_image(&rel) {
            PreviewKind::Image
        } else if is_markdown(&rel) {
            PreviewKind::Markdown
        } else {
            PreviewKind::Text
        };
        self.preview = Some(Preview {
            path: rel.clone(),
            lines: Arc::default(),
            highlight: None,
            markdown: None,
            line,
            error: None,
            kind,
            editing: false,
            office: None,
        });
        self.editor = None;
        self.office_status = None;
        if let Some(k) = office_kind {
            let work = cx.background_spawn(async move {
                let size = std::fs::metadata(&full).map_err(|e| e.to_string())?.len();
                if size > OFFICE_MAX {
                    return Err("This document is too large to preview".to_string());
                }
                office::load(&full, k)
            });
            self._load = Some(cx.spawn(async move |this, cx| {
                let result = work.await;
                let _ = this.update(cx, |p, cx| {
                    if let Some(preview) = p.preview.as_mut() {
                        match result {
                            Ok(doc) => preview.office = Some(OfficeView::new(doc, p.office_width.clone())),
                            Err(e) => preview.error = Some(e),
                        }
                    }
                    cx.notify();
                });
            }));
            cx.notify();
            return;
        }
        let work = cx.background_spawn(async move {
            let bytes = std::fs::read(&full).map_err(|e| e.to_string())?;
            if bytes.len() > PREVIEW_MAX {
                return Err("This file is too large to preview".to_string());
            }
            if is_image(&rel) {
                return Ok((Vec::new(), None, None));
            }
            if bytes.iter().take(8000).any(|b| *b == 0) {
                return Err("Binary file".to_string());
            }
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let md = is_markdown(&rel).then(|| markdown::parse(&text));
            let highlight = (text.len() <= HIGHLIGHT_MAX && md.is_none())
                .then(|| {
                    zeron_syntax::highlight(HighlightRequest { source: &text, path: Some(&rel), fence_tag: None }).ok()
                })
                .flatten()
                .map(Arc::new);
            let lines: Vec<String> = text.lines().map(|l| l.replace('\t', "    ")).collect();
            Ok((lines, highlight, md))
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |p, cx| {
                if let Some(preview) = p.preview.as_mut() {
                    match result {
                        Ok((lines, hl, md)) => {
                            preview.lines = Arc::new(lines);
                            preview.highlight = hl;
                            preview.markdown = md;
                        }
                        Err(e) => preview.error = Some(e),
                    }
                    if let Some(l) = preview.line {
                        p.preview_scroll.scroll_to_item(l.saturating_sub(4) as usize, ScrollStrategy::Top);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn close_preview(&mut self, cx: &mut Context<Self>) {
        self.preview = None;
        self.editor = None;
        cx.notify();
    }

    fn begin_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.preview.as_mut() else { return };
        p.editing = true;
        let text = p.lines.join("\n");
        let style = ui::input_style(Theme::of(cx));
        let editor = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_editor_style(style);
            s.set_value(text, window, cx);
            s
        });
        self.editor = Some(editor);
        cx.notify();
    }

    fn save_edit(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.preview.as_ref() else { return };
        let Some(root) = &self.root else { return };
        let Some(editor) = &self.editor else { return };
        let text = editor.read(cx).value().to_string();
        let path = root.join(&p.path);
        if std::fs::write(&path, &text).is_ok() {
            self.open_path(&p.path.clone(), None, cx);
        }
    }

    fn ask_agent(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.preview.as_ref() else { return };
        if matches!(p.kind, PreviewKind::Office(_)) {
            return self.office_ask(cx);
        }
        let instruction = self.editor.as_ref().map(|e| e.read(cx).value().to_string()).unwrap_or_default();
        let msg = if instruction.trim().is_empty() {
            format!("Edit `{}`. Propose a precise patch.", p.path)
        } else {
            format!("Edit `{}`:\n\n{}", p.path, instruction)
        };
        cx.emit(FilesEvent::AskAgent(msg));
    }

    fn office_view(&mut self) -> Option<&mut OfficeView> {
        self.preview.as_mut().and_then(|p| p.office.as_mut())
    }

    fn ensure_office_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.office_input.is_some() {
            return;
        }
        let style = ui::input_style(Theme::of(cx));
        let make = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            let style = style.clone();
            cx.new(|cx| {
                let mut s = InputState::new(window, cx).placeholder(placeholder);
                s.set_editor_style(style);
                s
            })
        };
        let edit = make("Select text, a cell or a field to edit it", window, cx);
        let ask = make("Ask the agent to change this document", window, cx);
        self._office_subs = vec![
            cx.subscribe(&edit, |this, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.office_apply(cx);
                }
            }),
            cx.subscribe(&ask, |this, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.office_ask(cx);
                }
            }),
        ];
        self.office_input = Some(edit);
        self.agent_input = Some(ask);
    }

    /// Select a paragraph, slide text, cell or form field for editing.
    pub(crate) fn office_select(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.office_input.clone();
        let Some(view) = self.office_view() else { return };
        let text = view.text_of(&target);
        view.selected = Some(target);
        if let Some(input) = input {
            input.update(cx, |s, cx| {
                s.set_value(text, window, cx);
                s.focus(window, cx);
            });
        }
        cx.notify();
    }

    /// Commit the edit field into the selected element (in memory).
    fn office_apply(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.office_input.as_ref().map(|i| i.read(cx).value().to_string()) else { return };
        let Some(view) = self.office_view() else { return };
        let Some(target) = view.selected.clone() else { return };
        view.apply(target, text);
        self.office_status = None;
        cx.notify();
    }

    pub(crate) fn office_toggle(&mut self, field: usize, cx: &mut Context<Self>) {
        if let Some(view) = self.office_view() {
            view.toggle_field(field);
            cx.notify();
        }
    }

    pub(crate) fn office_sheet(&mut self, sheet: usize, cx: &mut Context<Self>) {
        if let Some(view) = self.office_view() {
            view.sheet = sheet;
            view.selected = None;
            view.rebuild_grid();
            cx.notify();
        }
    }

    fn office_save(&mut self, cx: &mut Context<Self>) {
        // Typed but not yet applied: apply it first.
        self.office_apply(cx);
        let Some(root) = self.root.clone() else { return };
        let Some(p) = self.preview.as_ref() else { return };
        let Some(view) = p.office.as_ref() else { return };
        let rel = p.path.clone();
        match view.save(&root.join(&rel)) {
            Ok(()) => {
                self.open_path(&rel, None, cx);
                self.office_status = Some("Saved".into());
            }
            Err(e) => self.office_status = Some(format!("Could not save: {e}")),
        }
        cx.notify();
    }

    fn office_revert(&mut self, cx: &mut Context<Self>) {
        if let Some(rel) = self.preview.as_ref().map(|p| p.path.clone()) {
            self.open_path(&rel, None, cx);
        }
    }

    /// Hand the document (and the selection, if any) to the thread's agent.
    fn office_ask(&mut self, cx: &mut Context<Self>) {
        let instruction = self.agent_input.as_ref().map(|i| i.read(cx).value().to_string()).unwrap_or_default();
        self.office_ask_with(&instruction, cx);
    }

    fn office_ask_with(&mut self, instruction: &str, cx: &mut Context<Self>) {
        let Some(p) = self.preview.as_ref() else { return };
        let PreviewKind::Office(kind) = p.kind else { return };
        let skill = kind.skill();
        let mut msg = format!(
            "Edit `{}` using Keplorayx's {skill} skill (call the dove_skills tool with name \"{skill}\" for its instructions and script).",
            p.path
        );
        if let Some((view, sel)) = p.office.as_ref().and_then(|v| v.selected.as_ref().map(|s| (v, s))) {
            msg.push_str(&format!("\n\nSelected in Keplorayx: {} — \"{}\"", view.label(sel), view.text_of(sel)));
        }
        msg.push_str("\n\n");
        msg.push_str(if instruction.trim().is_empty() {
            "Propose a precise improvement and make it."
        } else {
            instruction.trim()
        });
        cx.emit(FilesEvent::AskAgent(msg));
    }

    fn office_bar(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let view = self.preview.as_ref().and_then(|p| p.office.as_ref());
        let label = view.and_then(|v| v.selected.as_ref().map(|s| v.label(s)));
        let dirty = view.map_or(0, OfficeView::dirty);
        let field = |input: &Entity<InputState>| {
            div()
                .flex_1()
                .min_w_0()
                .h(px(28.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .rounded(px(radius::RADIUS))
                .bg(t.wash(0.05))
                .border_1()
                .border_color(t.border)
                .text_size(ty::BODY)
                .child(Input::new(input))
        };
        let weak = cx.weak_entity();
        let on = move |f: fn(&mut FilesPanel, &mut Context<FilesPanel>)| {
            let weak = weak.clone();
            move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut gpui::App| {
                let _ = weak.update(cx, f);
            }
        };
        let quick_actions: &[(&str, &str)] = &[
            (
                "Polish the writing",
                "Proofread and polish the text in this document while preserving structure and formatting.",
            ),
            ("Summarize", "Summarize the key points and data in this document."),
            ("Fill the blanks", "Fill in any missing or placeholder fields/cells with sensible, realistic values."),
            (
                "Restyle",
                "Make this document look professionally designed: consistent headings, spacing and colors, without changing its content.",
            ),
        ];
        div()
            .flex_none()
            .px(px(12.0))
            .py(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .border_t_1()
            .border_color(t.border)
            .bg(t.wash(0.015))
            .when_some(label.clone().zip(self.office_input.as_ref()), |d, (label, input)| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_none()
                                .max_w(px(140.0))
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(radius::RADIUS))
                                .bg(t.wash(0.06))
                                .truncate()
                                .font_family(t.font_mono.clone())
                                .text_size(ty::MICRO)
                                .text_color(t.text_muted)
                                .child(label),
                        )
                        .child(field(input))
                        .child(ui::button_ghost("office-apply", "Apply", t).on_click(on(Self::office_apply))),
                )
            })
            .when(label.is_none(), |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(ty::CAPTION)
                        .text_color(t.text_faint)
                        .child("Click any text to edit it in place, or ask the agent:")
                        .child(div().flex_1())
                        .children(quick_actions.iter().enumerate().map(|(i, (chip, prompt))| {
                            let weak = cx.weak_entity();
                            let prompt = (*prompt).to_string();
                            div()
                                .id(SharedString::from(format!("office-qa-{i}")))
                                .h(px(22.0))
                                .px(px(7.0))
                                .flex()
                                .items_center()
                                .rounded(px(radius::fit(22.0)))
                                .border_1()
                                .border_color(t.border)
                                .bg(t.wash(0.03))
                                .cursor_pointer()
                                .text_size(ty::MICRO)
                                .text_color(t.text_muted)
                                .hover(|s| s.bg(t.wash(0.07)).text_color(t.text))
                                .child(*chip)
                                .on_click(move |_, _, cx| {
                                    let prompt = prompt.clone();
                                    let _ = weak.update(cx, |v, cx| v.office_ask_with(&prompt, cx));
                                })
                        })),
                )
            })
            .when_some(self.agent_input.as_ref(), |d, input| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(field(input))
                        .child(ui::button_ghost("office-ask", "Ask agent", t).on_click(on(Self::office_ask))),
                )
            })
            .when(dirty == 0 && self.office_status.is_some(), |d| {
                d.child(
                    div()
                        .text_size(ty::META)
                        .text_color(t.text_muted)
                        .child(self.office_status.clone().unwrap_or_default()),
                )
            })
            .into_any_element()
    }

    fn render_preview(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if matches!(self.preview.as_ref().map(|p| &p.kind), Some(PreviewKind::Office(_))) {
            self.ensure_office_inputs(window, cx);
        }
        let p = self.preview.as_ref().expect("preview open");
        let full = self.root.as_ref().map(|r| r.join(&p.path));
        let rel_for_copy = p.path.clone();
        let line_count = p.lines.len();
        let office_head = match (&p.kind, p.office.as_ref()) {
            (PreviewKind::Office(kind), Some(view)) => Some((*kind, view.doc.summary(), view.dirty())),
            (PreviewKind::Office(kind), None) => Some((*kind, "Opening…".to_string(), 0)),
            _ => None,
        };
        if let Some((kind, summary, dirty)) = office_head {
            let name = std::path::Path::new(&p.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| p.path.clone());
            let weak = cx.weak_entity();
            let header = div()
                .h(px(52.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(10.0))
                .border_b_1()
                .border_color(t.border)
                .child(ui::icon_button("prev-back", icons::ARROW_LEFT, "Back to files", t, {
                    let weak = weak.clone();
                    move |_, _, cx| {
                        let _ = weak.update(cx, |v, cx| v.close_preview(cx));
                    }
                }))
                .child(kit::avatar_tile(kind.short(), kind.hue(), 30.0, t).text_size(ty::MICRO))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().text_size(ty::BODY).text_color(t.text).truncate().child(name))
                        .child(
                            div()
                                .text_size(ty::CAPTION)
                                .text_color(t.text_faint)
                                .truncate()
                                .child(format!("{} · {summary}", kind.label())),
                        ),
                )
                .when(dirty > 0, |d| {
                    let (w1, w2) = (weak.clone(), weak.clone());
                    d.child(kit::badge(format!("{dirty} unsaved"), t.warning))
                        .child(ui::button_ghost("office-revert-top", "Revert", t).on_click(move |_, _, cx| {
                            let _ = w1.update(cx, |v, cx| v.office_revert(cx));
                        }))
                        .child(ui::button_primary("office-save-top", "Save", t).on_click(move |_, _, cx| {
                            let _ = w2.update(cx, |v, cx| v.office_save(cx));
                        }))
                })
                .child(ui::icon_button("prev-copy-path", icons::COPY, "Copy relative path", t, move |_, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(rel_for_copy.clone()));
                }))
                .when_some(full.clone(), |d, full| {
                    d.child(ui::icon_button(
                        "prev-open",
                        icons::ARROW_UP_RIGHT,
                        "Open in its app",
                        t,
                        move |_, _, cx| {
                            cx.open_with_system(&full);
                        },
                    ))
                });
            let body = match p.office.as_ref() {
                Some(view) => office::view::render(view, t, cx),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(ty::SMALL)
                    .text_color(t.text_faint)
                    .child("Opening…")
                    .into_any_element(),
            };
            let body = match &p.error {
                Some(e) => kit::empty_state(icons::DOCUMENT, "No preview", e, t).into_any_element(),
                None => body,
            };
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(header)
                .child(div().flex_1().min_h_0().bg(t.wash(0.03)).child(body))
                .child(self.office_bar(t, cx))
                .into_any_element();
        }
        let header = div()
            .h(px(36.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .border_b_1()
            .border_color(t.border)
            .child(ui::icon_button("prev-back", icons::ARROW_LEFT, "Back to files", t, {
                let weak = cx.weak_entity();
                move |_, _, cx| {
                    let _ = weak.update(cx, |v, cx| v.close_preview(cx));
                }
            }))
            .child(file_type_icon(&p.path, false, t).size(px(14.0)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ty::SMALL)
                    .font_family(t.font_mono.clone())
                    .child(p.path.clone()),
            )
            .when(matches!(p.kind, PreviewKind::Text | PreviewKind::Markdown) && line_count > 0, |d| {
                d.child(div().text_size(ty::MICRO).text_color(t.text_faint).child(format!("{line_count} lines")))
            })
            .child(ui::icon_button("prev-copy-path", icons::COPY, "Copy relative path", t, move |_, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(rel_for_copy.clone()));
            }))
            .when(!matches!(p.kind, PreviewKind::Image | PreviewKind::Office(_)), |d| {
                d.child(ui::icon_button("prev-edit", icons::PEN, "Edit this file", t, {
                    let weak = cx.weak_entity();
                    move |_, window, cx| {
                        let _ = weak.update(cx, |v, cx| v.begin_edit(window, cx));
                    }
                }))
            })
            .when(!matches!(p.kind, PreviewKind::Image), |d| {
                d.child(ui::icon_button("prev-ask", icons::BOT, "Ask the agent to edit", t, {
                    let weak = cx.weak_entity();
                    move |_, _, cx| {
                        let _ = weak.update(cx, |v, cx| v.ask_agent(cx));
                    }
                }))
            })
            .when_some(full.clone(), |d, full| {
                d.child(ui::icon_button("prev-open", icons::ARROW_UP_RIGHT, "Open in editor", t, move |_, _, cx| {
                    cx.open_with_system(&full);
                }))
            });
        if p.editing {
            let editor = self.editor.clone();
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(header)
                .child(div().flex_1().min_h_0().p(px(8.0)).children(editor.as_ref().map(Input::new)))
                .child(
                    div()
                        .h(px(40.0))
                        .px(px(10.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .border_t_1()
                        .border_color(t.border)
                        .child(ui::button_primary("file-save", "Save", t).on_click({
                            let weak = cx.weak_entity();
                            move |_, _, cx| {
                                let _ = weak.update(cx, |v, cx| v.save_edit(cx));
                            }
                        }))
                        .child(ui::button_ghost("file-ask", "Ask agent instead", t).on_click({
                            let weak = cx.weak_entity();
                            move |_, _, cx| {
                                let _ = weak.update(cx, |v, cx| v.ask_agent(cx));
                            }
                        })),
                )
                .into_any_element();
        }
        let office_kind = matches!(p.kind, PreviewKind::Office(_));
        let body: AnyElement = if let Some(e) = &p.error {
            kit::empty_state(icons::DOCUMENT, "No preview", e, t).into_any_element()
        } else if office_kind {
            match p.office.as_ref() {
                Some(view) => office::view::render(view, t, cx),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(ty::SMALL)
                    .text_color(t.text_faint)
                    .child("Opening…")
                    .into_any_element(),
            }
        } else if matches!(p.kind, PreviewKind::Image) {
            let src = full.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            let picture = markdown_extra::preview_image(&src)
                .map(|img| {
                    gpui::img(img).max_w_full().max_h_full().object_fit(gpui::ObjectFit::Contain).into_any_element()
                })
                .unwrap_or_else(|| {
                    gpui::img(SharedString::from(src))
                        .max_w_full()
                        .max_h_full()
                        .object_fit(gpui::ObjectFit::Contain)
                        .into_any_element()
                });
            div()
                .id("file-image")
                .size_full()
                .p(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .child(picture)
                .into_any_element()
        } else if matches!(p.kind, PreviewKind::Markdown) {
            let blocks = p.markdown.clone().unwrap_or_default();
            div()
                .id("file-md")
                .size_full()
                .overflow_y_scroll()
                .p(px(16.0))
                .child(markdown::render("file-md", &blocks, t, window))
                .into_any_element()
        } else {
            let lines = p.lines.clone();
            let hl = p.highlight.clone();
            let target = p.line;
            let gutter = (lines.len().max(1).ilog10() + 1) as f32 * 8.0 + 16.0;
            uniform_list("file-preview", lines.len(), move |range, window, cx| {
                let t = Theme::of(cx);
                let mut font = window.text_style().font();
                font.family = t.font_mono.clone();
                range
                    .map(|i| {
                        let line = &lines[i];
                        let spans = hl.as_ref().and_then(|h| h.lines.get(i));
                        let text: AnyElement = match spans.filter(|s| !s.is_empty() && !line.is_empty()) {
                            Some(spans) => {
                                let run = |len: usize, color| gpui::TextRun {
                                    len,
                                    font: font.clone(),
                                    color,
                                    background_color: None,
                                    underline: None,
                                    strikethrough: None,
                                };
                                let mut runs: Vec<TextRun> = Vec::new();
                                let mut at = 0usize;
                                for s in spans {
                                    let (a, b) = (s.range.start.min(line.len()), s.range.end.min(line.len()));
                                    if a < at || a >= b {
                                        continue;
                                    }
                                    if a > at {
                                        runs.push(run(a - at, t.text));
                                    }
                                    runs.push(run(b - a, t.syntax(s.kind)));
                                    at = b;
                                }
                                if at < line.len() {
                                    runs.push(run(line.len() - at, t.text));
                                }
                                StyledText::new(SharedString::from(line.clone())).with_runs(runs).into_any_element()
                            }
                            None => div()
                                .child(if line.is_empty() { " ".to_string() } else { line.clone() })
                                .into_any_element(),
                        };
                        let current = target == Some(i as u32 + 1);
                        div()
                            .h(px(19.0))
                            .flex()
                            .when(current, |d| d.bg(t.accent.opacity(0.12)))
                            .font_family(t.font_mono.clone())
                            .text_size(ty::META)
                            .child(
                                div()
                                    .w(px(gutter))
                                    .flex_none()
                                    .pr(px(8.0))
                                    .text_right()
                                    .text_color(t.text_faint)
                                    .child((i + 1).to_string()),
                            )
                            .child(div().flex_1().whitespace_nowrap().overflow_hidden().child(text))
                            .into_any_element()
                    })
                    .collect()
            })
            .track_scroll(&self.preview_scroll)
            .flex_1()
            .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(div().flex_1().min_h_0().flex().flex_col().child(body))
            .when(office_kind && p.error.is_none(), |d| d.child(self.office_bar(t, cx)))
            .into_any_element()
    }
}

/// Zeron's polychrome file-type icon (vscode-symbols) for a file or folder.
pub(crate) fn file_type_icon(path: &str, dir: bool, t: &Theme) -> gpui::Img {
    use crate::file_icons::{FileIconIdentity, icon};
    let identity = if dir { FileIconIdentity::directory(path, false) } else { FileIconIdentity::file(path) };
    icon(identity, t.appearance)
}

impl Render for FilesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("FilesPanel");
        let t = Theme::of(cx).clone();
        if self.preview.is_some() {
            return self.render_preview(&t, window, cx);
        }
        let file_count = self.files.len();
        let search = div()
            .m(px(8.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .flex_1()
                    .h(px(30.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .rounded(px(radius::RADIUS))
                    .bg(t.wash(0.05))
                    .border_1()
                    .border_color(t.border)
                    .text_size(ty::BODY)
                    .child(icon(icons::MAGNIFER).size(px(14.0)).text_color(t.text_faint))
                    .child(div().flex_1().child(Input::new(&self.search)))
                    .when(file_count > 0, |d| {
                        d.child(div().text_size(ty::MICRO).text_color(t.text_faint).child(format!("{file_count}")))
                    }),
            )
            .child(ui::icon_button(
                "files-refresh",
                icons::REFRESH,
                "Refresh workspace files",
                &t,
                cx.listener(|this, _, _, cx| this.refresh(cx)),
            ));

        let entries = self.visible.clone();
        let marks = self.git_marks.clone();
        let weak = cx.weak_entity();
        let tree: AnyElement = if self.root.is_none() {
            kit::empty_state(icons::FOLDER, "No workspace", "Open a thread to browse its files.", &t).into_any_element()
        } else if entries.is_empty() {
            div()
                .px(px(14.0))
                .py(px(6.0))
                .text_size(ty::BODY)
                .text_color(t.text_faint)
                .child(if self.query.is_empty() { "Empty folder" } else { "No matches" })
                .into_any_element()
        } else {
            let collapsed = self.collapsed.clone();
            uniform_list("files-tree", entries.len(), move |range, _, cx| {
                let t = Theme::of(cx);
                range
                    .map(|i| {
                        let e = &entries[i];
                        let weak = weak.clone();
                        let path = e.path.clone();
                        let dir = e.dir;
                        let open = dir && !collapsed.contains(&e.path);
                        let mark = marks.get(&e.path).copied();
                        let mark_color = match mark {
                            Some('?' | 'A') => t.success,
                            Some('D') => t.danger,
                            Some('•') => t.warning.opacity(0.8),
                            Some(_) => t.warning,
                            None => t.text_faint,
                        };
                        div()
                            .id(("file-row", i))
                            .h(px(26.0))
                            .pl(px(10.0 + e.depth as f32 * 12.0))
                            .pr(px(8.0))
                            .mx(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .rounded(px(radius::RADIUS))
                            .text_size(ty::BODY)
                            .text_color(t.text_muted)
                            .cursor_pointer()
                            .hover(|s| s.bg(t.wash(0.07)))
                            .child(if dir {
                                icon(if open { icons::ALT_ARROW_DOWN } else { icons::ALT_ARROW_RIGHT })
                                    .size(px(12.0))
                                    .text_color(t.text_faint)
                                    .into_any_element()
                            } else {
                                div().w(px(12.0)).into_any_element()
                            })
                            .child(file_type_icon(&e.name, dir, t).size(px(14.0)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .when(mark.is_some_and(|m| m != '•'), |d| d.text_color(mark_color))
                                    .when(mark == Some('D'), |d| d.line_through())
                                    .child(e.name.clone()),
                            )
                            .when_some(mark, |d, m| {
                                d.child(if m == '•' {
                                    ui::dot(mark_color, 5.0).into_any_element()
                                } else {
                                    div()
                                        .w(px(12.0))
                                        .text_right()
                                        .text_size(ty::MICRO)
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(mark_color)
                                        .child(if m == '?' { "U".to_string() } else { m.to_string() })
                                        .into_any_element()
                                })
                            })
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |p, cx| {
                                    if dir { p.toggle_dir(&path, cx) } else { p.open_path(&path, None, cx) }
                                });
                            })
                            .into_any_element()
                    })
                    .collect()
            })
            .flex_1()
            .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(t.text)
            .child(search)
            .child(div().flex_1().min_h_0().flex().flex_col().child(tree))
            .into_any_element()
    }
}
