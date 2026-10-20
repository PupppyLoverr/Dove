//! Settings → Memory: Supermemory as every agent's long-term memory.
//!
//! Sign-in (through the Integrations catalog entry, so the hub and the
//! recall/capture path share one token), the switches `dove_core::memory`
//! reads (memory, auto-recall, auto-capture, learn from corrections, per
//! project), never-send path globs, and the memories themselves: browse,
//! search and forget per scope. Also the first-sign-in warm-up (past
//! threads, project docs, merged PRs) and the weekly consolidation, which
//! finds duplicates and drafts skills from repeated workflows.

use std::sync::Arc;

use dove_core::Engine;
use dove_core::memory::{self, Memory, Report, Scope};
use dove_core::store::Project;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::design::{size, space, ty};
use crate::kit::controls::{self as ctl, SelectOption, SelectState, Tone};
use crate::kit::page;
use crate::pages::integrations::IntegrationsView;
use crate::theme::Theme;
use crate::ui;

pub struct MemoryView {
    engine: Arc<Engine>,
    integrations: Entity<IntegrationsView>,
    /// `who_am_i`, once asked; `None` while asking or signed out.
    who: Option<Result<String, String>>,
    asking: bool,
    outbox: usize,
    never_send: Entity<InputState>,
    query: Entity<InputState>,
    /// Scope choices: the user scope, then each project with its tag.
    projects: Vec<(Project, Option<String>)>,
    scope: usize,
    sel_scope: SelectState,
    results: Option<Result<Vec<Memory>, String>>,
    searching: bool,
    /// A long job (warm-up, consolidation) and what it last said.
    busy: Option<&'static str>,
    note: Option<String>,
    report: Option<Report>,
    /// The Supermemory server's config and sign-in state last seen.
    seen: String,
    _subs: Vec<Subscription>,
}

impl MemoryView {
    pub fn new(
        engine: Arc<Engine>,
        integrations: Entity<IntegrationsView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let globs = engine
            .store
            .setting(memory::NEVER_SEND)
            .unwrap_or_else(|| memory::DEFAULT_NEVER_SEND.to_string())
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let never_send = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder(".env*, secrets/**, *.pem");
            s.set_editor_style(style.clone());
            s.set_value(globs, window, cx);
            s
        });
        let query = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Search memories (empty: most recent)");
            s.set_editor_style(style.clone());
            s
        });
        let mut subs = vec![cx.observe(&integrations, |this, view, cx| {
            // Only when a sign-in started, finished or the server changed.
            let key = view.read(cx).catalog_key(crate::supermemory::CATALOG_ID);
            if key != this.seen {
                this.seen = key;
                this.refresh_status(cx);
            }
        })];
        subs.push(cx.subscribe(&never_send, |this, s, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                let globs = s
                    .read(cx)
                    .value()
                    .split([',', '\n'])
                    .map(str::trim)
                    .filter(|g| !g.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                let _ = this.engine.store.set_setting(memory::NEVER_SEND, &globs);
            }
        }));
        subs.push(cx.subscribe(&query, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                this.search(cx);
            }
        }));
        let projects = engine.store.projects().unwrap_or_default().into_iter().map(|p| (p, None)).collect();
        let report = Report::load(engine.store.root());
        let mut view = Self {
            engine,
            integrations,
            who: None,
            asking: false,
            outbox: 0,
            never_send,
            query,
            projects,
            scope: 0,
            sel_scope: SelectState::default(),
            results: None,
            searching: false,
            busy: None,
            note: None,
            report,
            seen: String::new(),
            _subs: subs,
        };
        view.refresh_status(cx);
        view.load_tags(cx);
        view
    }

    fn connected(&self) -> bool {
        memory::backend().is_some()
    }

    /// Ask who is signed in and how many saves wait in the outbox.
    pub fn refresh_status(&mut self, cx: &mut Context<Self>) {
        if self.asking {
            return;
        }
        let root = self.engine.store.root().to_path_buf();
        self.asking = true;
        let task = cx.background_spawn(async move {
            // The integrations save registers the backend on another thread;
            // give it a moment after a sign-in.
            crate::supermemory::refresh(&root);
            let who = memory::backend().map(|b| b.who_am_i());
            (who, memory::Outbox::at(&root).len())
        });
        cx.spawn(async move |this, cx| {
            let (who, outbox) = task.await;
            let _ = this.update(cx, |v, cx| {
                v.asking = false;
                v.who = who;
                v.outbox = outbox;
                cx.notify();
            });
        })
        .detach();
    }

    /// Work out each project's container tag (runs git) for the scope menu.
    fn load_tags(&mut self, cx: &mut Context<Self>) {
        let paths: Vec<std::path::PathBuf> = self.projects.iter().map(|(p, _)| p.path.clone()).collect();
        let task = cx.background_spawn(async move {
            paths.iter().map(|p| p.is_dir().then(|| memory::project_tag(p))).collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let tags = task.await;
            let _ = this.update(cx, |v, cx| {
                for ((_, tag), t) in v.projects.iter_mut().zip(tags) {
                    *tag = t;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn current_scope(&self) -> Option<Scope> {
        match self.scope {
            0 => Some(Scope::User),
            n => self.projects.get(n - 1)?.1.clone().map(Scope::Project),
        }
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let (Some(b), Some(scope)) = (memory::backend(), self.current_scope()) else {
            self.results = Some(Err("Sign in to Supermemory first.".into()));
            cx.notify();
            return;
        };
        let q = self.query.read(cx).value().trim().to_string();
        self.searching = true;
        cx.notify();
        let task =
            cx.background_spawn(async move { if q.is_empty() { b.list(&scope, 1, 30) } else { b.search(&q, &scope, 20) } });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |v, cx| {
                v.searching = false;
                v.results = Some(r);
                cx.notify();
            });
        })
        .detach();
    }

    fn forget(&mut self, i: usize, cx: &mut Context<Self>) {
        let (Some(b), Some(scope)) = (memory::backend(), self.current_scope()) else { return };
        let Some(Ok(list)) = &self.results else { return };
        let Some(m) = list.get(i).cloned() else { return };
        let task = cx.background_spawn(async move { b.forget(&m, &scope) });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |v, cx| {
                match r {
                    Ok(()) => {
                        if let Some(Ok(list)) = &mut v.results
                            && i < list.len()
                        {
                            list.remove(i);
                        }
                    }
                    Err(e) => v.note = Some(format!("Could not forget it: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_flag(&mut self, key: &str, on: bool, cx: &mut Context<Self>) {
        let _ = self.engine.store.set_setting(key, if on { "true" } else { "false" });
        cx.notify();
    }

    /// Ingest past threads, project docs and merged PRs.
    fn warm_up(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("Reading past threads and project docs…");
        cx.notify();
        let store = self.engine.store.clone();
        let task = cx.background_spawn(async move {
            let docs = memory::backfill_docs(&store, true);
            let n = docs.len();
            let (sent, left) = memory::ingest(store.root(), &docs);
            let _ = store.set_setting(memory::BACKFILL_OFFERED, "true");
            (n, sent, left)
        });
        cx.spawn(async move |this, cx| {
            let (n, sent, left) = task.await;
            let _ = this.update(cx, |v, cx| {
                v.busy = None;
                v.note = Some(if left == 0 {
                    format!("Saved {sent} of {n} items to memory.")
                } else {
                    format!("Saved {sent} of {n}; {left} wait in the outbox and retry automatically.")
                });
                v.outbox = left;
                cx.notify();
            });
        })
        .detach();
    }

    fn consolidate(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("Reviewing memories…");
        cx.notify();
        let store = self.engine.store.clone();
        let task = cx.background_spawn(async move { memory::consolidate(&store) });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |v, cx| {
                v.busy = None;
                match r {
                    Ok(report) => {
                        v.note = Some(format!(
                            "Looked at {} memories: {} duplicates, {} skill ideas.",
                            report.scanned,
                            report.duplicate_count(),
                            report.skills.len()
                        ));
                        v.report = Some(report);
                    }
                    Err(e) => v.note = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn forget_duplicates(&mut self, cx: &mut Context<Self>) {
        let (Some(b), Some(mut report)) = (memory::backend(), self.report.clone()) else { return };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("Forgetting duplicates…");
        cx.notify();
        let root = self.engine.store.root().to_path_buf();
        let task = cx.background_spawn(async move {
            let mut done = 0;
            for (scope, _, copies) in &report.duplicates {
                for m in copies {
                    if b.forget(m, scope).is_ok() {
                        done += 1;
                    }
                }
            }
            report.duplicates.clear();
            let _ = std::fs::write(Report::path(&root), serde_json::to_string_pretty(&report).unwrap_or_default());
            (done, report)
        });
        cx.spawn(async move |this, cx| {
            let (done, report) = task.await;
            let _ = this.update(cx, |v, cx| {
                v.busy = None;
                v.note = Some(format!("Forgot {done} duplicate memories."));
                v.report = Some(report);
                cx.notify();
            });
        })
        .detach();
    }

    // ------------------------------------------------------------ render

    fn connection(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.integrations.read(cx).catalog_status(crate::supermemory::CATALOG_ID);
        let (title, detail) = match (&self.who, self.connected()) {
            (_, false) => ("Not connected".to_string(), "Sign in so every agent Dove runs remembers you and this project.".to_string()),
            (Some(Ok(who)), true) => (format!("Signed in as {who}"), "Recall and capture run through your Supermemory account.".into()),
            (Some(Err(e)), true) => ("Supermemory is not answering".into(), e.clone()),
            (None, true) => ("Connected".into(), "Checking the account…".into()),
        };
        let detail = match &status {
            Some((_, s)) => s.clone(),
            None if self.outbox > 0 => format!("{detail} {} saves wait in the outbox.", self.outbox),
            None => detail,
        };
        let action = if self.connected() {
            div()
                .flex()
                .gap(px(6.0))
                .child(
                    ctl::button_sm(t, "mem-refresh", "Check", Tone::Filled)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_status(cx))),
                )
                .child(ctl::button_sm(t, "mem-sign-out", "Sign out", Tone::Quiet).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.integrations
                            .update(cx, |v, cx| v.sign_out_catalog(crate::supermemory::CATALOG_ID, cx));
                        this.who = None;
                        cx.notify();
                    },
                )))
                .into_any_element()
        } else {
            ctl::button_sm(t, "mem-sign-in", "Sign in with Supermemory", Tone::Solid)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.integrations.update(cx, |v, cx| v.sign_in_catalog(crate::supermemory::CATALOG_ID, cx));
                }))
                .into_any_element()
        };
        page::card(t).child(
            page::row(t, true)
                .child(page::row_tile(t, icons::STAR, self.connected().then_some(t.success)))
                .child(page::row_text(t, title, detail))
                .child(action),
        )
    }

    fn switches(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let store = &self.engine.store;
        let on = store.flag(memory::ENABLED, true);
        let rows: [(&'static str, &'static str, &'static str, bool); 4] = [
            (memory::ENABLED, "Memory", "Recall and capture for every agent and harness.", on),
            (
                memory::RECALL,
                "Recall before each prompt",
                "Search memory with your prompt and give the agent what it finds (about 1.5k tokens).",
                store.flag(memory::RECALL, true),
            ),
            (
                memory::CAPTURE,
                "Save each turn",
                "Your prompt, the final answer and the edited files, with secrets redacted.",
                store.flag(memory::CAPTURE, true),
            ),
            (
                memory::CORRECTIONS,
                "Learn from corrections",
                "When you correct an agent or deny an action, remember it so no agent repeats it.",
                store.flag(memory::CORRECTIONS, true),
            ),
        ];
        let mut card = page::card(t);
        for (i, (key, title, detail, value)) in rows.into_iter().enumerate() {
            let enabled = i == 0 || on;
            let toggle = ctl::on_toggle(ctl::toggle(t, SharedString::from(format!("mem-{key}")), value), cx, move |this, _, cx| {
                this.set_flag(key, !value, cx)
            });
            let toggle = if enabled { toggle } else { ctl::disabled(toggle, "Memory is off") };
            card = card.child(page::setting_row(t, i == 0, title, detail, toggle));
        }
        card
    }

    fn projects_card(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut card = page::card(t);
        if self.projects.is_empty() {
            return card.child(page::row(t, true).child(page::row_text(t, "No projects yet", "")));
        }
        for (i, (p, tag)) in self.projects.iter().enumerate() {
            let key = memory::project_key(p.id);
            let value = self.engine.store.flag(&key, true);
            let detail = tag.clone().unwrap_or_else(|| p.path.display().to_string());
            let toggle = ctl::on_toggle(
                ctl::toggle(t, SharedString::from(format!("mem-project-{i}")), value),
                cx,
                move |this, _, cx| this.set_flag(&key, !value, cx),
            );
            card = card.child(page::setting_row(t, i == 0, p.name.clone(), detail, toggle));
        }
        card
    }

    fn privacy(&self, t: &Theme) -> impl IntoElement {
        page::card(t).child(
            page::row(t, true)
                .child(page::row_text(
                    t,
                    "Never send",
                    "Path globs left out of captured turns and the warm-up (comma separated). Credential-shaped strings are always redacted.",
                ))
                .child(
                    div()
                        .w(px(300.0))
                        .h(px(size::FIELD))
                        .px(px(10.0))
                        .flex()
                        .items_center()
                        .rounded(px(8.0))
                        .bg(t.wash(0.06))
                        .text_size(ty::BODY)
                        .child(Input::new(&self.never_send)),
                ),
        )
    }

    fn browse(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut options = vec![SelectOption::new("You (all projects)")];
        options.extend(self.projects.iter().map(|(p, tag)| {
            let o = SelectOption::new(p.name.clone());
            match tag {
                Some(tag) => o.detail(tag.clone()),
                None => o.detail("working out its tag…"),
            }
        }));
        let scope = ctl::select::<Self>("mem-scope", "Scope", |s| &mut s.sel_scope)
            .options(options, self.scope)
            .width(220.0)
            .on_select(|this, i, _, cx| {
                this.scope = i;
                this.results = None;
                this.search(cx);
            })
            .render(t, &self.sel_scope, cx);
        let head = div()
            .flex()
            .items_center()
            .gap(px(space::SM))
            .px(px(space::LG))
            .py(px(space::MD))
            .child(scope)
            .child(div().flex_1().rounded(px(8.0)).bg(t.wash(0.06)).child(page::search_field(t, &self.query)))
            .child(
                ctl::button_sm(t, "mem-search", if self.searching { "Searching…" } else { "Search" }, Tone::Filled)
                    .on_click(cx.listener(|this, _, _, cx| this.search(cx))),
            );
        let body = match &self.results {
            None => div()
                .px(px(space::LG))
                .pb(px(space::MD))
                .text_size(ty::META)
                .text_color(t.text_muted)
                .child("Pick a scope and search, or press Search for the most recent memories.")
                .into_any_element(),
            Some(Err(e)) => page::error_strip(t, e.clone()).into_any_element(),
            Some(Ok(list)) if list.is_empty() => div()
                .px(px(space::LG))
                .pb(px(space::MD))
                .text_size(ty::META)
                .text_color(t.text_muted)
                .child("Nothing here yet.")
                .into_any_element(),
            Some(Ok(list)) => div()
                .flex()
                .flex_col()
                .children(list.iter().enumerate().map(|(i, m)| {
                    page::row(t, i == 0)
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(180.0))
                                .text_size(ty::BODY)
                                .text_color(t.text)
                                .child(m.text.chars().take(600).collect::<String>()),
                        )
                        .when_some(m.score, |d, s| d.child(page::badge(t, format!("{:.0}%", s * 100.0))))
                        .child(
                            ctl::button_sm(t, SharedString::from(format!("mem-forget-{i}")), "Forget", Tone::Quiet)
                                .on_click(cx.listener(move |this, _, _, cx| this.forget(i, cx))),
                        )
                }))
                .into_any_element(),
        };
        page::card(t).child(head).child(body)
    }

    fn learning(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let warmed = self.engine.store.flag(memory::BACKFILL_OFFERED, false);
        let last = self
            .engine
            .store
            .setting(memory::CONSOLIDATED_AT)
            .and_then(|s| s.parse::<i64>().ok())
            .map(|at| format!("Last run {}.", ui::relative_time(at)))
            .unwrap_or_else(|| "Runs weekly in the background.".into());
        let dups = self.report.as_ref().map(Report::duplicate_count).unwrap_or(0);
        let mut card = page::card(t)
            .child(page::setting_row(
                t,
                true,
                "Warm up memory",
                if warmed {
                    "Done once. Run again to pick up new threads, docs and merged PRs."
                } else {
                    "Import past Dove threads, each project's README / ARCHITECTURE / CLAUDE.md / AGENTS.md and recent merged PRs."
                },
                ctl::button_sm(t, "mem-warm", if warmed { "Run again" } else { "Warm up" }, Tone::Filled)
                    .on_click(cx.listener(|this, _, _, cx| this.warm_up(cx))),
            ))
            .child(page::setting_row(
                t,
                false,
                "Consolidate",
                format!("Find duplicate memories and workflows worth a skill. {last}"),
                ctl::button_sm(t, "mem-consolidate", "Run now", Tone::Filled)
                    .on_click(cx.listener(|this, _, _, cx| this.consolidate(cx))),
            ));
        if dups > 0 {
            card = card.child(page::setting_row(
                t,
                false,
                format!("{dups} duplicate memories"),
                "The newest copy of each stays.",
                ctl::button_sm(t, "mem-dups", "Forget duplicates", Tone::Danger)
                    .on_click(cx.listener(|this, _, _, cx| this.forget_duplicates(cx))),
            ));
        }
        if let Some(report) = &self.report {
            let dir = memory::suggested_skills_dir(self.engine.store.root());
            for (i, s) in report.skills.iter().enumerate() {
                let path = dir.join(&s.name);
                card = card.child(page::setting_row(
                    t,
                    false,
                    format!("Suggested skill: {}", s.name),
                    format!("Seen in {} memories: {}", s.seen, s.steps.join(" · ")),
                    ctl::button_sm(t, SharedString::from(format!("mem-skill-{i}")), "Review draft", Tone::Outlined)
                        .child(icon(icons::ARROW_UP_RIGHT).size(px(12.0)))
                        .on_click(cx.listener(move |_, _, _, cx| cx.open_with_system(&path))),
                ));
            }
        }
        card
    }
}

impl Render for MemoryView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx).clone();
        let status = self
            .busy
            .map(|b| page::info_strip(&t, b).into_any_element())
            .or_else(|| self.note.clone().map(|n| page::info_strip(&t, n).into_any_element()));
        let connected = self.connected();
        div()
            .flex()
            .flex_col()
            .gap(px(space::XL))
            .child(self.connection(&t, cx))
            .children(status)
            .child(page::section(&t, "Behaviour", self.switches(&t, cx)))
            .when(connected, |d| d.child(page::section(&t, "Memories", self.browse(&t, cx))))
            .child(page::section(&t, "Projects", self.projects_card(&t, cx)))
            .child(page::section(&t, "Privacy", self.privacy(&t)))
            .when(connected, |d| d.child(page::section(&t, "Self-improving", self.learning(&t, cx))))
    }
}
