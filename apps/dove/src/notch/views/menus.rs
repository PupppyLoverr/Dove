//! The island's menus: one at a time, drawn inside the island next to what
//! opened it (the window is clipped to the island, so nothing may hang
//! outside it), closed by Esc or a click elsewhere.

use super::*;

impl Notch {
    /// Opens a menu by the pointer that clicked (`at`, window coordinates).
    pub(crate) fn open_menu(&mut self, at: gpui::Point<gpui::Pixels>, items: Vec<MenuItem>, cx: &mut Context<Self>) {
        if items.is_empty() {
            return;
        }
        let g = Geo::from_array(self.springs.targets());
        let mut m = Menu { x: 0.0, y: 0.0, items, hovered: None };
        let h = m.height();
        let (px_, py_) = (f32::from(at.x) - g.x(), f32::from(at.y) - g.y);
        m.x = (px_ - MENU_W + 24.0).clamp(8.0, (g.w - MENU_W - 8.0).max(8.0));
        m.y = if py_ + 10.0 + h <= g.h - 8.0 { py_ + 10.0 } else { (py_ - h - 10.0).max(8.0) };
        self.menu = Some(m);
        self.retarget(cx);
        cx.notify();
    }

    pub(crate) fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let m = self.menu.as_ref()?;
        let mut col = div()
            .id("menu")
            .absolute()
            .left(px(m.x))
            .top(px(m.y))
            .w(px(MENU_W))
            .p(px(6.0))
            .rounded(px(14.0))
            .bg(tk::FLOAT)
            .border_1()
            .border_color(tk::RIM)
            .shadow_lg()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        for (i, item) in m.items.iter().enumerate() {
            let Some(action) = item.action.clone() else {
                col = col.child(div().h(px(MENU_HEAD)).px(px(8.0)).flex().items_end().pb(px(3.0)).child(section(item.label.clone())));
                continue;
            };
            let on = m.hovered == Some(i);
            let color = if item.danger { tk::RED } else { tk::TEXT };
            col = col.child(
                div()
                    .id(("menu-item", i))
                    .h(px(MENU_ROW))
                    .px(px(8.0))
                    .rounded(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .cursor_pointer()
                    .when(on, |d| d.bg(tk::WASH_HOVER))
                    .hover(|s| s.bg(tk::WASH_HOVER))
                    .text_size(px(tk::T12))
                    .text_color(color)
                    .when_some(item.checked, |d, c| {
                        d.child(div().w(px(14.0)).child(if c {
                            icon(icons::CHECK).size(px(14.0)).text_color(tk::TEXT).into_any_element()
                        } else {
                            div().into_any_element()
                        }))
                    })
                    .when_some(item.icon, |d, path| {
                        d.child(icon(path).size(px(15.0)).text_color(if item.danger { tk::RED } else { tk::DIM }))
                    })
                    .child(div().flex_1().min_w_0().truncate().child(item.label.clone()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.run_menu(action.clone(), window, cx))),
            );
        }
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.menu = None;
                        this.retarget(cx);
                        cx.notify();
                    }),
                )
                .child(col)
                .into_any_element(),
        )
    }

    /// The header's menu (right-click it): keep open, settings, hide.
    pub(crate) fn header_menu(&self) -> Vec<MenuItem> {
        vec![
            MenuItem::check("Keep open", self.machine.pinned, MenuAction::Pin),
            MenuItem::act("Settings", icons::SETTINGS, MenuAction::Settings),
            MenuItem::act("Hide for an hour", icons::EYE_CLOSED, MenuAction::HideHour),
        ]
    }

    pub(crate) fn agent_menu(&self, id: TaskId, running: bool) -> Vec<MenuItem> {
        let mut items = vec![
            MenuItem::act("Message", icons::CHAT_ROUND_LINE, MenuAction::AgentMessage(id)),
            MenuItem::act("Open in Dove", icons::ARROW_UP_RIGHT, MenuAction::AgentOpen(id)),
        ];
        if let Some(pr) = self.snap.prs.iter().find(|p| p.task == id && p.state == "open") {
            items.push(MenuItem::act(format!("Open PR #{}", pr.number), icons::GIT_MERGE, MenuAction::OpenUrl(pr.url.clone())));
            if pr.checks == "passing" && pr.mergeable {
                items.push(MenuItem::act(format!("Merge #{}", pr.number), icons::GIT_MERGE, MenuAction::Merge(id, pr.number)));
            }
        }
        if running {
            items.push(MenuItem::danger("Stop", icons::STOP, MenuAction::AgentStop(id)));
        }
        items
    }

    pub(crate) fn new_agent_menu(&self) -> Vec<MenuItem> {
        let mut items = vec![MenuItem::head("Agent")];
        for k in &self.snap.installed {
            items.push(MenuItem::check(k.label(), self.new_agent.kind == Some(*k), MenuAction::NewKind(*k)));
        }
        if !self.snap.projects.is_empty() {
            items.push(MenuItem::head("Project"));
            for (id, name) in self.snap.projects.iter().take(6) {
                items.push(MenuItem::check(name.clone(), self.new_agent.project == Some(*id), MenuAction::NewProject(*id)));
            }
        }
        items.push(MenuItem::head("Where"));
        items.push(MenuItem::check("Its own worktree", self.new_agent.worktree, MenuAction::NewWorktree(!self.new_agent.worktree)));
        items
    }

    pub(crate) fn tile_menu(&self, item: TrayItem) -> Vec<MenuItem> {
        match item {
            TrayItem::File(i) => vec![
                MenuItem::act("Send to agent", icons::SPARKLE_AGENT, MenuAction::ShelfStart(i)),
                MenuItem::act("Copy path", icons::COPY, MenuAction::ShelfCopy(i)),
                MenuItem::danger("Remove", icons::TRASH_BIN_MINIMALISTIC, MenuAction::ShelfRemove(i)),
            ],
            TrayItem::Clip(i) => {
                let Some(c) = self.clips.get(i) else { return Vec::new() };
                let mut items = Vec::new();
                if let Some(t) = c.as_text() {
                    if super::super::data::looks_like_error(t) {
                        items.push(MenuItem::act("Ask an agent to fix this", icons::CODE_SCAN, MenuAction::TileFix(i)));
                    }
                    items.push(MenuItem::act("Send to agent", icons::SPARKLE_AGENT, MenuAction::TileSend(i)));
                }
                items.push(MenuItem::check("Pinned", c.pinned, MenuAction::ClipPin(i)));
                items
            }
        }
    }

    pub(crate) fn tray_menu(&self) -> Vec<MenuItem> {
        let mut items = vec![MenuItem::head("Show")];
        for (f, label) in [
            (TrayFilter::All, "Everything"),
            (TrayFilter::Files, "Files"),
            (TrayFilter::Text, "Text"),
            (TrayFilter::Images, "Images"),
            (TrayFilter::Links, "Links"),
            (TrayFilter::Pinned, "Pinned"),
        ] {
            items.push(MenuItem::check(label, self.tray_filter == f, MenuAction::Filter(f)));
        }
        items.push(MenuItem::head("Actions"));
        items.push(MenuItem::act("Capture the window behind", icons::SCREEN_CAPTURE, MenuAction::Capture));
        if self.clips.iter().any(|c| !c.pinned) {
            items.push(MenuItem::danger("Clear unpinned clips", icons::TRASH_BIN_MINIMALISTIC, MenuAction::ClearClips));
        }
        items
    }

    pub(crate) fn note_menu(&self, id: i64) -> Vec<MenuItem> {
        let pinned = self.notes.iter().any(|n| n.id == id && n.pinned);
        vec![
            MenuItem::check("Pinned to the island", pinned, MenuAction::NotePin(id)),
            MenuItem::act("Send as prompt", icons::SPARKLE_AGENT, MenuAction::NoteSend(id)),
            MenuItem::danger("Delete", icons::TRASH_BIN_MINIMALISTIC, MenuAction::NoteDelete(id)),
        ]
    }

    pub(crate) fn timer_menu(&self) -> Vec<MenuItem> {
        let mut items = Vec::new();
        let running: Vec<&Agent> = self.snap.agents.iter().filter(|a| a.status == TaskStatus::Running).take(4).collect();
        if !running.is_empty() || self.timer_link.is_some() {
            items.push(MenuItem::head("End early when"));
            for a in running {
                items.push(MenuItem::check(format!("“{}” finishes", a.title), self.timer_link == Some(a.id), MenuAction::LinkTimer(Some(a.id))));
            }
            if self.timer_link.is_some() {
                items.push(MenuItem::act("Unlink", icons::CLOSE, MenuAction::LinkTimer(None)));
            }
        }
        if self.timer.active() {
            items.push(MenuItem::act("Reset", icons::REFRESH, MenuAction::ResetTimer));
        }
        items
    }

    pub(crate) fn port_menu(port: u16, pid: u32) -> Vec<MenuItem> {
        vec![
            MenuItem::act("Copy URL", icons::COPY, MenuAction::PortCopy(port)),
            MenuItem::danger("Stop the server", icons::STOP, MenuAction::PortStop(pid)),
        ]
    }

    pub(crate) fn clear_notices_menu() -> Vec<MenuItem> {
        vec![MenuItem::danger("Clear all", icons::TRASH_BIN_MINIMALISTIC, MenuAction::ClearNotices)]
    }

    /// Deny, then Allow with its options (Always; all of a burst of similar
    /// safe requests) behind a chevron. Risky requests only open in Dove;
    /// questions with a few options answer right here.
    pub(crate) fn decision_row(&self, id: TaskId, req: &Pending, key: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let row = div().flex().items_center().justify_end().gap(px(6.0));
        if req.question {
            if let Some(q) = req.questions.first().filter(|q| req.questions.len() == 1 && !q.multi && q.options.len() <= 4) {
                let mut r = row;
                for (i, opt) in q.options.iter().enumerate() {
                    let (rid, opt) = (req.request_id.clone(), opt.clone());
                    let last = i + 1 == q.options.len();
                    let el = if last { primary((key, 10 + i), opt.clone()) } else { pill((key, 10 + i), opt.clone(), false) };
                    r = r.child(el.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.answer_pick(id, rid.clone(), opt.clone(), false, cx))));
                }
                return r.into_any_element();
            }
            return row
                .child(primary((key, 1usize), "Answer").on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.open_page(Page::Agent(id), Some(window), cx)
                })))
                .into_any_element();
        }
        let rid = req.request_id.clone();
        let row = row.child(pill((key, 2usize), "Deny", false).tooltip(tip("Deny · D")).on_click(cx.listener({
            let rid = rid.clone();
            move |this: &mut Notch, _: &ClickEvent, _, cx| this.deny(id, rid.clone(), cx)
        })));
        if !req.safe {
            return row
                .child(primary((key, 5usize), "Review in Dove").on_click(cx.listener(move |this, _: &ClickEvent, _, _| {
                    this.command(ToEngine::Open { task: Some(id) })
                })))
                .into_any_element();
        }
        // A burst of the same safe tool can be allowed together.
        let similar: Vec<String> = self
            .snap
            .agents
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.pending.iter().filter(|p| p.safe && !p.question && p.tool == req.tool).map(|p| p.request_id.clone()).collect())
            .unwrap_or_default();
        let mut options = vec![MenuItem::act("Always allow", icons::CHECK, MenuAction::AllowAlways(id, rid.clone()))];
        if similar.len() > 1 {
            options.push(MenuItem::act(
                format!("Allow all {} {} requests", similar.len(), req.tool),
                icons::CHECKLIST,
                MenuAction::AllowAll(id, similar),
            ));
        }
        row.child(
            div()
                .flex()
                .items_center()
                .h(px(tk::CONTROL_H))
                .rounded_full()
                .bg(tk::WHITE)
                .child(
                    div()
                        .id((key, 4usize))
                        .h_full()
                        .pl(px(14.0))
                        .pr(px(8.0))
                        .flex()
                        .items_center()
                        .rounded_l_full()
                        .text_size(px(tk::T12))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tk::INK)
                        .cursor_pointer()
                        .hover(|s| s.bg(tk::WHITE_HOVER))
                        .tooltip(tip("Allow · A"))
                        .child("Allow")
                        .on_click(cx.listener({
                            let rid = rid.clone();
                            move |this, _: &ClickEvent, _, cx| this.decide(id, rid.clone(), PermissionDecision::AllowOnce, cx)
                        })),
                )
                .child(div().w(px(1.0)).h(px(14.0)).bg(tk::INK.opacity(0.18)))
                .child(
                    div()
                        .id((key, 6usize))
                        .h_full()
                        .pl(px(6.0))
                        .pr(px(9.0))
                        .flex()
                        .items_center()
                        .rounded_r_full()
                        .cursor_pointer()
                        .hover(|s| s.bg(tk::WHITE_HOVER))
                        .tooltip(tip("More ways to allow"))
                        .child(icon(icons::ALT_ARROW_DOWN).size(px(13.0)).text_color(tk::INK))
                        .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| this.open_menu(ev.position(), options.clone(), cx))),
                ),
        )
        .into_any_element()
    }
}
