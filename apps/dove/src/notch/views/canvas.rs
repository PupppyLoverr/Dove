//! The island itself: the glass, its shape and motion, the slim capsule and
//! its bubbles, banners and popups, the expanded frame (header, page, Undo),
//! flying shared elements, and the keyboard.

use super::*;

impl Notch {
    pub(crate) fn t(&self) -> f32 {
        self.epoch.elapsed().as_secs_f32()
    }

    pub(crate) fn key(&self) -> u64 {
        self.page_at.duration_since(self.epoch).as_millis() as u64
    }

    pub(crate) fn dot(&self, color: Hsla, size: f32, breathing: bool) -> Div {
        let a = if breathing { breathe(self.t()) } else { 1.0 };
        div().size(px(size)).flex_none().rounded_full().bg(color.opacity(a))
    }

    /// Four bars that follow the real output level (when Windows reports
    /// one), else a gentle rhythm while playing.
    pub(crate) fn eq(&self, height: f32) -> Div {
        let playing = self.media.as_ref().is_some_and(|m| m.playing);
        let peak = self.audio_state.peak;
        let t = self.t();
        let mut row = div().h(px(height)).flex().items_end().gap(px(2.0)).flex_none();
        for (speed, phase) in [(7.1f32, 0.0f32), (9.3, 1.3), (6.2, 2.1), (8.4, 0.7)] {
            let wobble = 0.5 + 0.5 * (t * speed + phase).sin();
            let k = if !playing {
                0.0
            } else if peak > 0.001 {
                (peak.sqrt() * (0.55 + 0.45 * wobble)).clamp(0.0, 1.0)
            } else {
                0.25 + 0.6 * wobble
            };
            let h = (3.0 + (height - 3.0) * k).max(3.0);
            row = row.child(div().w(px(2.5)).h(px(h)).rounded_full().bg(tk::GREEN));
        }
        row
    }

    pub(crate) fn art(&self, size: f32, radius: f32) -> AnyElement {
        match self.media.as_ref().and_then(|m| m.art.clone()) {
            Some(a) => img(a).size(px(size)).flex_none().rounded(px(radius)).object_fit(ObjectFit::Cover).into_any_element(),
            None => div()
                .size(px(size))
                .flex_none()
                .rounded(px(radius))
                .bg(tk::WASH_HOVER)
                .flex()
                .items_center()
                .justify_center()
                .child(icon(icons::MUSIC_NOTE).size(px((size * 0.45).round())).text_color(tk::DIM))
                .into_any_element(),
        }
    }

    /// A shared element is flying toward this kind's place: the real one
    /// waits underneath, invisible.
    pub(crate) fn hero_flying(&self, kind: HeroKind) -> bool {
        self.hero.is_some_and(|h| h.kind == kind && h.at.elapsed().as_secs_f32() * 1000.0 < tk::HERO_MS)
    }

    // ----- slim -----------------------------------------------------------------

    pub(crate) fn slim_items(&self) -> Vec<SlimItem> {
        let mut items = Vec::new();
        if self.prefs.slim_agents {
            let mut groups: Vec<(HarnessKind, usize, bool)> = Vec::new();
            let mut add = |kind: HarnessKind, needs: bool| match groups.iter_mut().find(|g| g.0 == kind) {
                Some(g) => {
                    g.1 += 1;
                    g.2 |= needs;
                }
                None => groups.push((kind, 1, needs)),
            };
            let (mut done, mut total) = (0u32, 0u32);
            for a in self.snap.agents.iter().filter(|a| a.active()) {
                // An approval a focus timer holds doesn't ask for you yet.
                let held = !a.pending.is_empty() && a.pending.iter().all(|p| self.held(p));
                add(a.kind, a.needs_you() && !held);
                if let Some((d, t)) = a.todos {
                    done += d;
                    total += t;
                }
            }
            for f in self.snap.flock.iter().filter(|f| f.working || f.needs_input) {
                add(f.kind, f.needs_input);
            }
            if !groups.is_empty() {
                let progress = (total > 0).then(|| done as f32 / total as f32);
                items.push(SlimItem::Agents { groups, progress });
            }
            if let Some(pr) = self.snap.prs.iter().find(|p| p.state == "open" && matches!(p.checks.as_str(), "pending" | "failing")) {
                items.push(SlimItem::Pr { checks: pr.checks.clone() });
            }
        }
        if self.prefs.slim_music && self.media.as_ref().is_some_and(|m| m.playing) {
            items.push(SlimItem::Music);
        }
        if self.prefs.slim_timer && self.timer.active() {
            items.push(SlimItem::Timer);
        }
        if self.prefs.slim_note
            && let Some(n) = self.notes.iter().find(|n| n.pinned)
        {
            items.push(SlimItem::Note(n.title()));
        }
        // Needs you always leads; otherwise scrolling picks the primary.
        let needs = matches!(items.first(), Some(SlimItem::Agents { groups, .. }) if groups.iter().any(|g| g.2));
        if !needs && !items.is_empty() {
            let k = self.primary % items.len();
            items.rotate_left(k);
        }
        items
    }

    /// The slim width (one of three) and how many bubbles.
    pub(crate) fn slim_size(&self) -> (f32, usize) {
        let items = self.slim_items();
        let width = match items.first() {
            None | Some(SlimItem::Timer) => tk::SLIM_SINGLE,
            Some(SlimItem::Agents { groups, .. }) => {
                let marks = groups.len().min(3);
                if marks >= 3 || (marks == 2 && groups.iter().any(|g| g.2)) { tk::SLIM_WIDE } else { tk::SLIM_DUAL }
            }
            Some(SlimItem::Pr { .. } | SlimItem::Music | SlimItem::Note(_)) => tk::SLIM_DUAL,
        };
        let width = if self.context_menu { tk::SLIM_WIDE } else { width };
        (width, items.len().saturating_sub(1).min(2))
    }

    fn slim_leading(&self, item: &SlimItem) -> AnyElement {
        match item {
            SlimItem::Agents { groups, progress } => {
                let needs = groups.iter().any(|g| g.2);
                let color = if needs { tk::AMBER } else { tk::BLUE };
                let mut stack = div().flex().items_center();
                for (i, (kind, _, n)) in groups.iter().take(3).enumerate() {
                    let face = div()
                        .relative()
                        .when(i > 0, |d| d.ml(px(-6.0)))
                        .child(
                            div()
                                .size(px(22.0))
                                .rounded_full()
                                .bg(gpui::hsla(240.0 / 360.0, 0.06, 0.16, 1.0))
                                .border_1()
                                .border_color(tk::HAIRLINE)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(mark(*kind, 12.0)),
                        )
                        .when(*n, |d| d.child(self.dot(tk::AMBER, 6.0, true).absolute().top(px(-1.0)).right(px(-1.0))));
                    stack = stack.child(face);
                }
                match progress {
                    Some(p) => progress_ring(*p, 30.0, 2.0, color, stack.into_any_element()),
                    None => stack.into_any_element(),
                }
            }
            SlimItem::Pr { checks } => pr_ring(checks, 26.0),
            SlimItem::Music => {
                if self.hero_flying(HeroKind::Art) {
                    div().size(px(22.0)).into_any_element()
                } else {
                    self.art(22.0, 7.0)
                }
            }
            SlimItem::Timer => progress_ring(
                self.timer.progress(),
                24.0,
                2.0,
                tk::AMBER,
                icon(icons::TIMER).size(px(11.0)).text_color(tk::AMBER).into_any_element(),
            ),
            SlimItem::Note(_) => icon(icons::NOTE_TEXT).size(px(15.0)).text_color(tk::TEXT).into_any_element(),
        }
    }

    fn slim_trailing(&self, item: &SlimItem) -> AnyElement {
        if let Some((text, _)) = &self.moment {
            return div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .text_size(px(tk::T12))
                .font_weight(FontWeight::MEDIUM)
                .text_color(tk::GREEN)
                .child(icon(icons::CHECK).size(px(13.0)).text_color(tk::GREEN))
                .child(text.clone())
                .into_any_element();
        }
        let phrase = |text: String, color: Hsla| {
            div().text_size(px(tk::T12)).font_weight(FontWeight::MEDIUM).text_color(color).truncate().child(text).into_any_element()
        };
        match item {
            SlimItem::Agents { groups, .. } => {
                let n: usize = groups.iter().map(|g| g.1).sum();
                if groups.iter().any(|g| g.2) {
                    phrase("Needs you".into(), tk::AMBER)
                } else {
                    phrase(if n == 1 { "Working".into() } else { format!("{n} working") }, tk::BLUE)
                }
            }
            SlimItem::Pr { checks } => {
                if checks == "pending" {
                    phrase("Checks running".into(), tk::BLUE)
                } else {
                    phrase("Checks failed".into(), tk::RED)
                }
            }
            SlimItem::Music => self.eq(14.0).into_any_element(),
            SlimItem::Timer => roll(
                &mins_secs(self.timer.remaining()),
                "slim-timer",
                tk::T12,
                if self.timer.running() { tk::TEXT } else { tk::DIM },
                FontWeight::MEDIUM,
            )
            .into_any_element(),
            SlimItem::Note(title) => phrase(title.clone(), tk::DIM),
        }
    }

    fn render_slim(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.context_menu {
            return div()
                .size_full()
                .px(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(4.0))
                .child(link("slim-hide", "Hide for an hour").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.hide_for_an_hour(cx))))
                .child(link("slim-settings", "Settings").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.context_menu = false;
                    this.command(ToEngine::OpenSettings);
                    cx.notify();
                })))
                .into_any_element();
        }
        let items = self.slim_items();
        let (left, right) = match items.first() {
            None => (
                icon(icons::DOVE_LOGO).size(px(15.0)).text_color(tk::TEXT.opacity(0.6)).into_any_element(),
                div()
                    .text_size(px(tk::T12))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(tk::DIM.opacity(0.6))
                    .child(super::super::clock_text())
                    .into_any_element(),
            ),
            Some(first) => (self.slim_leading(first), self.slim_trailing(first)),
        };
        let count: usize = match items.first() {
            Some(SlimItem::Agents { groups, .. }) => groups.iter().map(|g| g.1).sum(),
            _ => 0,
        };
        div()
            .id("slim")
            .size_full()
            .pl(px(8.0))
            .pr(px(14.0))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.0))
            .when(count > 1, |d| d.tooltip(tip(format!("{count} agents"))))
            .child(div().flex_none().child(left))
            .child(div().min_w_0().flex().justify_end().child(right))
            .into_any_element()
    }

    fn render_bubble(&self, index: usize) -> AnyElement {
        let items = self.slim_items();
        let Some(item) = items.get(index + 1) else { return div().into_any_element() };
        let inner = match item {
            SlimItem::Music => self.art(22.0, 11.0),
            SlimItem::Timer => progress_ring(
                self.timer.progress(),
                24.0,
                2.0,
                tk::AMBER,
                icon(icons::TIMER).size(px(11.0)).text_color(tk::AMBER).into_any_element(),
            ),
            SlimItem::Agents { groups, progress } => {
                let needs = groups.iter().any(|g| g.2);
                let m = groups.first().map(|(k, _, _)| mark(*k, 13.0)).unwrap_or_else(|| div().into_any_element());
                progress_ring(progress.unwrap_or(-1.0), 26.0, 2.0, if needs { tk::AMBER } else { tk::BLUE }, m)
            }
            SlimItem::Pr { checks } => pr_ring(checks, 26.0),
            SlimItem::Note(_) => icon(icons::NOTE_TEXT).size(px(14.0)).text_color(tk::TEXT).into_any_element(),
        };
        div().size_full().flex().items_center().justify_center().child(inner).into_any_element()
    }

    // ----- banners and popups -------------------------------------------------------

    /// The image area of the screenshot popup (canvas x, y, w, h).
    fn shot_rect(&self, shot: &Shot) -> (f32, f32, f32, f32) {
        let x0 = (tk::CANVAS_W - tk::EXPANDED_W) / 2.0 + 14.0;
        let max_w = tk::EXPANDED_W - 28.0;
        let (w, h) = (shot.size.0.max(1) as f32, shot.size.1.max(1) as f32);
        let (mut dw, mut dh) = (max_w, max_w * h / w);
        if dh > 250.0 {
            dh = 250.0;
            dw = 250.0 * w / h;
        }
        (x0 + (max_w - dw) / 2.0, tk::TOP_GAP + 14.0, dw, dh)
    }

    /// The banner's size: wide for a screenshot, taller with a preview.
    pub(crate) fn banner_size(&self) -> (f32, f32) {
        let menu = self.menu.as_ref().map(|m| m.bottom() + 10.0).unwrap_or(0.0);
        if let Some((_, p)) = self.approval() {
            let h = match &p.preview {
                _ if p.question && p.questions.len() == 1 => 96.0 + 8.0,
                Some(Preview::Diff { lines, .. }) if !lines.is_empty() => 96.0 + 14.0 * lines.len().min(3) as f32 + 12.0,
                Some(Preview::Command(_)) => 96.0 + 28.0,
                _ => tk::BANNER_H,
            };
            return (tk::BANNER_W, h.max(menu));
        }
        let (w, h) = match self.popup() {
            Some(Popup::Shot(shot)) => (tk::EXPANDED_W, self.shot_rect(shot).3 + 14.0 + 12.0 + tk::CONTROL_H + 14.0),
            Some(Popup::Usage { .. }) => (tk::BANNER_W, 104.0),
            Some(Popup::FixError { .. }) => (tk::BANNER_W, 112.0),
            Some(_) => (tk::BANNER_W, tk::BANNER_H),
            None => (tk::BANNER_W, tk::FLASH_H),
        };
        (w, h.max(menu))
    }

    pub(crate) fn render_alert(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some((agent, req)) = self.approval() {
            let (id, req, kind, title) = (agent.id, req.clone(), agent.kind, agent.title.clone());
            let more = agent.pending.iter().filter(|p| !self.dismissed.contains(&p.request_id)).count();
            let line = if req.question {
                req.questions.first().map(|q| q.question.clone()).unwrap_or_else(|| format!("{} asks: {}", kind.label(), req.summary))
            } else {
                format!("{} wants to {}", kind.label(), req.tool.to_lowercase())
            };
            return div()
                .size_full()
                .px(px(14.0))
                .py(px(12.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .size(px(32.0))
                                .rounded(px(10.0))
                                .bg(if req.safe { tk::WASH } else { tk::AMBER.opacity(0.16) })
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(if req.safe {
                                    mark(kind, 16.0)
                                } else {
                                    icon(icons::DANGER_TRIANGLE).size(px(16.0)).text_color(tk::AMBER).into_any_element()
                                }),
                        )
                        .child(
                            div()
                                .id("banner-body")
                                .flex_1()
                                .min_w_0()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_page(Page::Agent(id), Some(window), cx)))
                                .child(div().text_size(px(tk::T13)).font_weight(FontWeight::SEMIBOLD).text_color(tk::TEXT).truncate().child(title))
                                .child(div().text_size(px(tk::T12)).text_color(tk::DIM).truncate().child(line)),
                        )
                        .when(more > 1, |d| d.child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(format!("1 of {more}")))),
                )
                .when_some(req.preview.clone().filter(|_| !req.question), |d, p| d.child(preview(&p, 3)))
                .child(div().flex_1())
                .child(self.decision_row(id, &req, "banner", cx))
                .into_any_element();
        }
        if let Some(popup) = self.popup().cloned() {
            return self.render_popup(popup, cx);
        }
        let Some(flash) = &self.flash else { return div().into_any_element() };
        let (glyph, color): (AnyElement, Hsla) = match (flash.kind, flash.agent) {
            (k, Some(kind)) => (avatar(kind, 28.0, None), if k == FlashKind::Failed { tk::RED } else { tk::GREEN }),
            (FlashKind::Timer, None) => (icon(icons::TIMER).size(px(17.0)).text_color(tk::AMBER).into_any_element(), tk::AMBER),
            (FlashKind::Failed, None) => (icon(icons::DANGER_TRIANGLE).size(px(17.0)).text_color(tk::RED).into_any_element(), tk::RED),
            _ => (icon(icons::CHECK).size(px(17.0)).text_color(tk::GREEN).into_any_element(), tk::GREEN),
        };
        let task = flash.task;
        let left = 1.0 - flash.at.elapsed().as_secs_f32() / flash.until.duration_since(flash.at).as_secs_f32().max(0.1);
        div()
            .id("flash")
            .relative()
            .size_full()
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.flash = None;
                match task {
                    Some(id) => this.open_page(Page::Agent(id), Some(window), cx),
                    None => cx.notify(),
                }
            }))
            .child(glyph)
            .child(div().flex_1().min_w_0().truncate().text_size(px(tk::T13)).font_weight(FontWeight::MEDIUM).text_color(tk::TEXT).child(flash.text.clone()))
            .child(self.dot(color, 7.0, false))
            // The countdown along the bottom.
            .child(div().absolute().bottom_0().left(px(18.0)).right(px(18.0)).h(px(2.0)).child(bar(left, color.opacity(0.7), 2.0)))
            .into_any_element()
    }

    /// A popup's head: a glyph in a tinted plate, a title and a line.
    fn popup_head(glyph: &'static str, color: Hsla, title: impl Into<SharedString>, line: impl Into<SharedString>) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .size(px(32.0))
                    .flex_none()
                    .rounded(px(10.0))
                    .bg(color.opacity(0.16))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(glyph).size(px(17.0)).text_color(color)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(tk::T13)).font_weight(FontWeight::SEMIBOLD).text_color(tk::TEXT).truncate().child(title.into()))
                    .child(div().text_size(px(tk::T12)).text_color(tk::DIM).truncate().child(line.into())),
            )
    }

    fn popup_shell(head: Div, body: Option<AnyElement>, actions: Div) -> AnyElement {
        div()
            .size_full()
            .px(px(14.0))
            .py(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(head)
            .children(body)
            .child(div().flex_1())
            .child(actions.flex().items_center().justify_end().gap(px(6.0)))
            .into_any_element()
    }

    fn render_popup(&mut self, popup: Popup, cx: &mut Context<Self>) -> AnyElement {
        let dismiss = |label: &'static str, cx: &mut Context<Self>| {
            link("popup-dismiss", label).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.dismiss_popup(cx)))
        };
        let target = self.target_agent().and_then(|id| self.snap.agents.iter().find(|a| a.id == id)).map(|a| (a.id, a.kind));
        match popup {
            Popup::Usage { meter, .. } => {
                let color = if meter.used >= 95.0 { tk::RED } else { tk::AMBER };
                let resets = meter.resets_at.map(|s| format!("Resets {}", until(s * 1000))).unwrap_or_default();
                Self::popup_shell(
                    Self::popup_head(icons::CHART, color, format!("{} · {} at {:.0}%", meter.kind.label(), meter.label, meter.used), resets),
                    Some(bar(meter.used as f32 / 100.0, color, 4.0).into_any_element()),
                    div().child(primary("usage-ok", "Got it").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.dismiss_popup(cx)))),
                )
            }
            Popup::FixError { text } => {
                let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
                let label = match target {
                    Some((_, kind)) => format!("Fix with {}", kind.label()),
                    None => "Ask an agent".to_string(),
                };
                Self::popup_shell(
                    Self::popup_head(icons::CODE_SCAN, tk::RED, "You copied an error", "An agent can look into it"),
                    Some(
                        div()
                            .px(px(8.0))
                            .py(px(5.0))
                            .rounded(px(10.0))
                            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.22))
                            .font_family(tk::MONO)
                            .text_size(px(tk::T11))
                            .text_color(tk::TEXT)
                            .truncate()
                            .child(first)
                            .into_any_element(),
                    ),
                    div().child(dismiss("Not now", cx)).child(primary("fix-error", label).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.dismiss_popup(cx);
                        this.fix_error(text.clone(), Some(window), cx);
                    }))),
                )
            }
            Popup::Dropped { paths } => {
                let n = paths.len();
                let name = paths.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let title = if n == 1 { format!("{name} is in the tray") } else { format!("{n} files are in the tray") };
                let mention: String = paths.iter().map(|p| format!("@{} ", p.display())).collect();
                let send = target.map(|(id, kind)| {
                    let paths = paths.clone();
                    link("dropped-send", format!("Send to {}", kind.label())).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.dismiss_popup(cx);
                        this.drop_on_agent(id, &ExternalPaths(paths.clone().into()), cx);
                    }))
                });
                Self::popup_shell(
                    Self::popup_head(icons::DOCUMENT_ADD, tk::BLUE, title, "Hand them to an agent?"),
                    None,
                    div().children(send).child(primary("dropped-start", "Start an agent").on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            this.dismiss_popup(cx);
                            this.new_agent_with(mention.clone(), Some(window), cx);
                        },
                    ))),
                )
            }
            Popup::ServerDown { port, process } => {
                let prompt = format!(
                    "The dev server on localhost:{port} ({process}) stopped unexpectedly. Find out why from its output or logs, fix it, and start it again."
                );
                Self::popup_shell(
                    Self::popup_head(icons::SERVER, tk::RED, format!("localhost:{port} stopped"), format!("{process} exited")),
                    None,
                    div().child(dismiss("Dismiss", cx)).child(primary("server-fix", "Ask an agent").on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            this.dismiss_popup(cx);
                            match this.target_agent() {
                                Some(_) => this.send_to_agent(prompt.clone(), cx),
                                None => this.new_agent_with(prompt.clone(), Some(window), cx),
                            }
                        },
                    ))),
                )
            }
            Popup::Checks { pr } => {
                let passed = pr.checks == "passing";
                let (task, number, url) = (pr.task, pr.number, pr.url.clone());
                let act = if passed && pr.mergeable {
                    primary("checks-merge", "Merge").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.dismiss_popup(cx);
                        this.command(ToEngine::Merge { task, number });
                        this.toast("Merging…");
                    }))
                } else {
                    primary("checks-fix", "Ask to fix").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.dismiss_popup(cx);
                        let text = format!("The checks on PR #{number} failed. Look at the failures and fix them.");
                        this.command(ToEngine::Send { task, text });
                        this.toast("Sent");
                    }))
                };
                Self::popup_shell(
                    Self::popup_head(
                        icons::GIT_MERGE,
                        if passed { tk::GREEN } else { tk::RED },
                        if passed { format!("Checks passed on #{}", pr.number) } else { format!("Checks failed on #{}", pr.number) },
                        pr.title.clone(),
                    ),
                    None,
                    div()
                        .child(link("checks-open", "Open").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.open_url(&url);
                            this.dismiss_popup(cx);
                        })))
                        .child(act),
                )
            }
            Popup::Held { count } => Self::popup_shell(
                Self::popup_head(
                    icons::TIMER,
                    tk::AMBER,
                    if count == 1 { "1 approval waited for you".to_string() } else { format!("{count} approvals waited for you") },
                    "They held while your focus timer ran",
                ),
                None,
                div().child(primary("held-review", "Review").on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.dismiss_popup(cx);
                    this.open_page(Page::Tab(Tab::Agents), Some(window), cx);
                }))),
            ),
            Popup::Shot(shot) => self.render_shot(shot, cx),
        }
    }

    fn shot_mut(&mut self) -> Option<&mut Shot> {
        self.popups.iter_mut().find_map(|(p, _)| match p {
            Popup::Shot(s) => Some(s),
            _ => None,
        })
    }

    /// The screenshot popup: draw on it, then send it or keep it.
    fn render_shot(&mut self, shot: Shot, cx: &mut Context<Self>) -> AnyElement {
        let rect = self.shot_rect(&shot);
        let (x, y, w, h) = rect;
        let island_x = (tk::CANVAS_W - tk::EXPANDED_W) / 2.0;
        let frac = move |p: gpui::Point<gpui::Pixels>| {
            (((f32::from(p.x) - rect.0) / rect.2).clamp(0.0, 1.0), ((f32::from(p.y) - rect.1) / rect.3).clamp(0.0, 1.0))
        };
        let marks: Vec<Mark> = shot.marks.iter().copied().chain(shot.drawing).collect();
        let current = shot.tool;
        let tool_btn = |id: &'static str, glyph: &'static str, tool: ShotTool, name: &'static str, cx: &mut Context<Self>| {
            let on = current == tool;
            ibtn(id, glyph, 28.0, if on { tk::TEXT } else { tk::DIM }, name).when(on, |d| d.bg(tk::WASH_ON)).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    if let Some(s) = this.shot_mut() {
                        s.tool = tool;
                    }
                    cx.notify();
                },
            ))
        };
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .id("shot")
                    .absolute()
                    .left(px(x - island_x))
                    .top(px(y - tk::TOP_GAP))
                    .w(px(w))
                    .h(px(h))
                    .rounded(px(12.0))
                    .overflow_hidden()
                    .cursor_crosshair()
                    .child(img(shot.image.clone()).size_full().object_fit(ObjectFit::Fill))
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, _| {
                                let (bx, by) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                                let (bw, bh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                                let at = |f: (f32, f32)| point(px(bx + f.0 * bw), px(by + f.1 * bh));
                                for m in &marks {
                                    paint_mark(window, m.tool, at(m.from), at(m.to));
                                }
                            },
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            let f = frac(ev.position);
                            if let Some(s) = this.shot_mut() {
                                s.drawing = Some(Mark { tool: s.tool, from: f, to: f });
                            }
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                        if ev.pressed_button != Some(MouseButton::Left) {
                            return;
                        }
                        let f = frac(ev.position);
                        if let Some(s) = this.shot_mut()
                            && let Some(d) = &mut s.drawing
                        {
                            d.to = f;
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseUpEvent, _, cx| {
                            let f = frac(ev.position);
                            if let Some(s) = this.shot_mut()
                                && let Some(mut d) = s.drawing.take()
                            {
                                d.to = f;
                                if (d.to.0 - d.from.0).abs() + (d.to.1 - d.from.1).abs() > 0.01 {
                                    s.marks.push(d);
                                }
                            }
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(14.0))
                    .right(px(14.0))
                    .bottom(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(tool_btn("tool-arrow", icons::ARROW_UP_RIGHT, ShotTool::Arrow, "Arrow", cx))
                    .child(tool_btn("tool-box", icons::BOX_SELECT, ShotTool::Box, "Box", cx))
                    .child(tool_btn("tool-hl", icons::HIGHLIGHTER, ShotTool::Highlight, "Highlight", cx))
                    .when(!shot.marks.is_empty(), |d| {
                        d.child(ibtn("tool-undo", icons::UNDO, 28.0, tk::DIM, "Undo mark").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(s) = this.shot_mut() {
                                s.marks.pop();
                            }
                            cx.notify();
                        })))
                    })
                    .child(div().flex_1())
                    .child(link("shot-discard", "Discard").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.dismiss_popup(cx))))
                    .child(pill("shot-keep", "Keep", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.finish_shot(false, cx))))
                    .child(primary("shot-send", "Send to agent").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.finish_shot(true, cx)))),
            )
            .into_any_element()
    }

    // ----- the expanded frame ---------------------------------------------------------

    /// A rough height for some text, so pages can size the island first.
    fn text_height(text: &str, per_line: usize) -> f32 {
        let lines: usize = text.lines().map(|l| l.chars().count().div_ceil(per_line).max(1)).sum();
        lines.min(60) as f32 * 17.0
    }

    pub(crate) fn page_height(&self) -> f32 {
        const BOTTOM: f32 = 14.0;
        if !self.query.trim().is_empty() {
            let n = self.hits().len().max(1) as f32;
            return (tk::HEADER_H + 10.0 + n * 42.0 + BOTTOM).min(tk::EXPANDED_MAX_H);
        }
        let notice = if self.undo.is_some() || self.deny.is_some() || self.stopping.is_some() { 40.0 } else { 0.0 };
        let body: f32 = match &self.page {
            Page::Tab(Tab::Home) => {
                let cards = self.home_cards();
                if cards.is_empty() {
                    150.0
                } else {
                    let shown = cards.len().min(3);
                    let h: f32 = cards.iter().take(3).map(|c| c.height(self)).sum::<f32>() + tk::GAP * (shown as f32 - 1.0);
                    h + if cards.len() > 3 { 36.0 } else { 0.0 }
                }
            }
            Page::Tab(Tab::Agents) => {
                if self.snap.agents.is_empty() && self.snap.flock.is_empty() {
                    150.0
                } else {
                    let groups = [
                        self.snap.agents.iter().any(|a| a.needs_you()),
                        self.snap.agents.iter().any(|a| a.status == TaskStatus::Running && !a.needs_you()),
                        self.snap.agents.iter().any(|a| !a.active()),
                        !self.snap.flock.is_empty(),
                    ]
                    .iter()
                    .filter(|g| **g)
                    .count() as f32;
                    let rows = (self.snap.agents.len() + self.snap.flock.len()) as f32;
                    let limits = if self.snap.limits.iter().any(|m| m.used >= 50.0) { 30.0 } else { 0.0 };
                    34.0 + groups * 26.0 + rows * (tk::ROW_H + 2.0) + limits
                }
            }
            Page::Agent(id) => {
                let turns = self.turns_of(*id);
                let text: f32 = turns
                    .iter()
                    .map(|t| {
                        Self::text_height(&t.reply, 62)
                            + if t.user.is_empty() { 0.0 } else { Self::text_height(&t.user, 48) + 18.0 }
                            + if t.steps.is_empty() { 0.0 } else { 24.0 }
                    })
                    .sum();
                let sent: f32 = self.sent.iter().filter(|m| m.task == *id).map(|m| Self::text_height(&m.text, 48) + 30.0).sum();
                let waiting = self
                    .snap
                    .agents
                    .iter()
                    .find(|a| a.id == *id)
                    .and_then(|a| a.pending.iter().find(|p| !self.dismissed.contains(&p.request_id)))
                    .map(|p| if p.question { 120.0 } else if p.preview.is_some() { 160.0 } else { 100.0 })
                    .unwrap_or(0.0);
                let picker = if self.picking { (self.shelf.len().clamp(1, 5) as f32) * 30.0 + 22.0 } else { 0.0 };
                let attached = if self.attachments.is_empty() { 0.0 } else { 34.0 };
                (44.0 + (text + sent).max(40.0) + 20.0 + waiting + picker + attached + 50.0)
                    .clamp(220.0, tk::EXPANDED_MAX_H - tk::HEADER_H - BOTTOM)
            }
            Page::NewAgent => 40.0 + 12.0 + 22.0 + 12.0 + tk::CONTROL_H + 24.0,
            Page::Tab(Tab::Music) => {
                if self.media.is_some() {
                    204.0
                } else {
                    130.0
                }
            }
            Page::Tab(Tab::Tray) => {
                let n = self.tray_items().len();
                30.0 + 8.0 + if n == 0 { 110.0 } else { (n.div_ceil(2) as f32) * 92.0 }
            }
            Page::Tab(Tab::Notes) => 250.0f32.max(36.0 + self.notes.len() as f32 * 48.0),
            Page::Tab(Tab::Timer) => 232.0,
            Page::Tab(Tab::Alerts) => {
                if self.notices.is_empty() {
                    130.0
                } else {
                    34.0 + self.notices.len() as f32 * 44.0
                }
            }
            Page::Tab(Tab::Dev) => {
                if self.ports.is_empty() {
                    140.0
                } else {
                    10.0 + self.ports.len() as f32 * 50.0
                }
            }
        };
        let menu = self.menu.as_ref().map(|m| m.bottom() + 10.0).unwrap_or(0.0);
        (tk::HEADER_H + body + notice + BOTTOM).max(menu).clamp(170.0, tk::EXPANDED_MAX_H)
    }

    /// The header: the five tabs with a sliding puck (or a back arrow on a
    /// page without a tab), and the search, which grows into a field.
    fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        const SLOT: f32 = 32.0;
        const STEP: f32 = 36.0;
        let searching =
            !self.query.is_empty() || self.inputs.as_ref().is_some_and(|i| i.command.read(cx).focus_handle(cx).is_focused(window));
        let command = self.inputs.as_ref().map(|i| i.command.clone());
        let page_tab = tab_of(&self.page);
        let mut row = div().id("header").h(px(tk::HEADER_H)).px(px(10.0)).flex().items_center().gap(px(8.0)).on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                let items = this.header_menu();
                this.open_menu(ev.position, items, cx);
            }),
        );
        if !searching {
            if !page_tab.in_bar() {
                // A page without a tab: back, and its name.
                row = row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(ibtn("back", icons::ARROW_LEFT, 32.0, tk::DIM, "Back · Esc").on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| this.go_back(window, cx),
                        )))
                        .child(div().text_size(px(tk::T13)).font_weight(FontWeight::SEMIBOLD).text_color(tk::TEXT).child(page_tab.label())),
                );
            } else {
                let tabs = self.prefs.tabs();
                let at = tabs.iter().position(|t| *t == page_tab).unwrap_or(0) as f32;
                let from = self.tab_from.and_then(|f| tabs.iter().position(|t| *t == f)).map(|i| i as f32).unwrap_or(at);
                let p = (self.page_at.elapsed().as_secs_f32() * 1000.0 / tk::PUCK_MS).min(1.0);
                // Quick out, soft landing, no bounce.
                let e = 1.0 - (1.0 - p).powi(4);
                if p < 1.0 && !crate::motion::reduced_now() {
                    window.request_animation_frame();
                }
                let puck_x = if crate::motion::reduced_now() { at * STEP } else { (from + (at - from) * e) * STEP };
                let needs = self.approval().is_some();
                let mut strip = div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(STEP - SLOT))
                    .child(
                        div()
                            .absolute()
                            .left(px(puck_x))
                            .top_0()
                            .size(px(SLOT))
                            .rounded_full()
                            .bg(tk::WASH_HOVER)
                            .border_1()
                            .border_color(tk::CARD_EDGE),
                    );
                for (i, t) in tabs.iter().copied().enumerate() {
                    let on = t == page_tab;
                    strip = strip.child(
                        div()
                            .id(("tab", i))
                            .relative()
                            .size(px(SLOT))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(tk::WASH))
                            .tooltip(tip(format!("{} · {}", t.label(), i + 1)))
                            .child(icon(glyph(t)).size(px(tk::ICON_HEADER)).text_color(if on { tk::TEXT } else { tk::FAINT }))
                            .when(t == Tab::Agents && needs, |d| d.child(self.dot(tk::AMBER, 6.0, true).absolute().top(px(5.0)).right(px(5.0))))
                            .when(t == Tab::Alerts && self.unread_notices > 0, |d| {
                                d.child(self.dot(tk::BLUE, 6.0, false).absolute().top(px(5.0)).right(px(5.0)))
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_page(Page::Tab(t), Some(window), cx))),
                    );
                }
                row = row.child(strip);
            }
            row = row.child(div().flex_1());
            if let Some((text, at)) = self.toast.as_ref() {
                let t = (at.elapsed().as_secs_f32() / 0.16).min(1.0);
                row = row.child(
                    div()
                        .flex_none()
                        .px(px(10.0))
                        .h(px(24.0))
                        .rounded_full()
                        .bg(tk::GREEN.opacity(0.16))
                        .flex()
                        .items_center()
                        .text_size(px(tk::T11))
                        .text_color(tk::GREEN)
                        .opacity(t)
                        .relative()
                        .left(px((1.0 - t) * 8.0))
                        .child(text.clone()),
                );
            }
            if self.prefs.focus {
                row = row.child(div().id("focus-on").tooltip(tip("Focus mode is on")).child(icon(icons::MOON).size(px(15.0)).text_color(tk::AMBER)));
            }
            if self.machine.pinned {
                row = row.child(div().id("pinned").tooltip(tip("Kept open · Ctrl+P")).child(icon(icons::PIN).size(px(15.0)).text_color(tk::FAINT)));
            }
        }
        row.child(
            div()
                .id("command")
                .when(searching, |d| d.flex_1())
                .when(!searching, |d| d.w(px(32.0)).justify_center().cursor_pointer().tooltip(tip("Search or type a command")))
                .h(px(32.0))
                .px(px(if searching { 12.0 } else { 0.0 }))
                .rounded_full()
                .when(searching, |d| d.bg(tk::WASH_HOVER))
                .hover(|s| s.bg(tk::WASH_HOVER))
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(px(tk::T12))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.take_focus(cx);
                        this.focus_command = true;
                        cx.notify();
                    }),
                )
                .child(icon(icons::MAGNIFER).size(px(tk::ICON_ROW)).text_color(if searching { tk::FAINT } else { tk::DIM }))
                // The field stays (at no width) so focus can move into it.
                .children(command.map(|c| {
                    div().when(!searching, |d| d.w(px(0.0)).overflow_hidden()).when(searching, |d| d.flex_1().min_w_0()).child(Input::new(&c))
                })),
        )
        .into_any_element()
    }

    /// Esc, Backspace or the back arrow: where this page was opened from.
    pub(crate) fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let to = match (&self.page, self.back.take()) {
            (_, Some(p)) => p,
            (Page::Agent(_) | Page::NewAgent, None) => Page::Tab(Tab::Agents),
            _ => Page::Tab(Tab::Home),
        };
        self.open_page(to, Some(window), cx);
    }

    /// The Undo bar for the last destructive action (stop, deny, delete).
    fn undo_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let text = if let Some((id, _)) = self.stopping {
            let name = self.snap.agents.iter().find(|a| a.id == id).map(|a| a.kind.label()).unwrap_or("the agent");
            format!("Stopping {name}…")
        } else if self.deny.is_some() {
            "Denied".to_string()
        } else {
            match self.undo.as_ref().map(|u| &u.0) {
                Some(super::super::data::Undo::Note(..)) => "Note deleted".into(),
                Some(super::super::data::Undo::Clips(..)) => "Clipboard cleared".into(),
                Some(super::super::data::Undo::Shelf(..)) => "Removed from the tray".into(),
                None => return None,
            }
        };
        Some(
            div()
                .mt(px(8.0))
                .h(px(32.0))
                .px(px(12.0))
                .rounded_full()
                .bg(tk::WASH)
                .flex()
                .items_center()
                .text_size(px(tk::T12))
                .text_color(tk::DIM)
                .child(div().flex_1().child(text))
                .child(
                    div()
                        .id("undo")
                        .px(px(10.0))
                        .text_color(tk::TEXT)
                        .font_weight(FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .child("Undo")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.undo_last(cx))),
                )
                .into_any_element(),
        )
    }

    fn render_expanded(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let body = if !self.query.trim().is_empty() {
            self.page_hits(cx)
        } else {
            match self.page.clone() {
                Page::Tab(Tab::Home) => self.page_home(cx),
                Page::Tab(Tab::Agents) => self.page_agents(cx),
                Page::Agent(id) => self.page_agent(id, window, cx),
                Page::NewAgent => self.page_new(cx),
                Page::Tab(Tab::Music) => self.page_music(cx),
                Page::Tab(Tab::Tray) => self.page_tray(cx),
                Page::Tab(Tab::Notes) => self.page_notes(cx),
                Page::Tab(Tab::Timer) => self.page_timer(cx),
                Page::Tab(Tab::Alerts) => self.page_alerts(cx),
                Page::Tab(Tab::Dev) => self.page_dev(cx),
            }
        };
        // A page change cross-fades in, rising a little.
        let p = (self.page_at.elapsed().as_secs_f32() * 1000.0 / tk::PAGE_MS).min(1.0);
        let reduced = crate::motion::reduced_now();
        if p < 1.0 && !reduced {
            window.request_animation_frame();
        }
        let fade = if reduced { 1.0 } else { 1.0 - (1.0 - p).powi(2) };
        let undo = self.undo_bar(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.header(window, cx))
            .child(div().flex_1().min_h_0().px(px(tk::PAD)).opacity(fade).relative().top(px((1.0 - fade) * 3.0)).child(body))
            .children(undo.map(|u| div().px(px(tk::PAD)).child(u)))
            .child(div().h(px(14.0)))
            .into_any_element()
    }

    // ----- the canvas -------------------------------------------------------------

    /// The glow over the glass: amber while something needs you, green or red
    /// for a moment after a finish, else the album art's colour while music
    /// plays.
    fn glow(&self) -> Option<Hsla> {
        if self.approval().is_some() {
            return Some(tk::AMBER.opacity(tk::GLOW * (0.6 + 0.4 * breathe(self.t()))));
        }
        if let Some(f) = &self.flash {
            let c = match f.kind {
                FlashKind::Failed => tk::RED,
                FlashKind::Timer => tk::AMBER,
                FlashKind::Done => tk::GREEN,
            };
            return Some(c.opacity(tk::GLOW));
        }
        let m = self.media.as_ref().filter(|m| m.playing)?;
        let (r, g, b) = m.tint?;
        let c: Hsla = gpui::Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }.into();
        Some(c.opacity(tk::GLOW * 1.4))
    }

    /// `DOVE_NOTCH_TIMING=1`: logs how long a transition took to draw its
    /// first frame and to settle, and its worst gap between frames.
    fn note_frame(&mut self, now: Instant, moving: bool) {
        let Some(t) = &mut self.timing else { return };
        if t.what.is_empty() {
            return;
        }
        if t.first.is_none() {
            t.first = Some(now);
        }
        if let Some(last) = t.last_frame {
            t.max_gap = t.max_gap.max(now.duration_since(last));
        }
        t.last_frame = Some(now);
        if !moving {
            let first = t.first.map(|f| f.duration_since(t.at).as_millis()).unwrap_or(0);
            tracing::info!(
                target: "dove::notch",
                "notch timing: {}: first frame {first} ms, settled {} ms, worst frame gap {} ms",
                t.what,
                now.duration_since(t.at).as_millis(),
                t.max_gap.as_millis()
            );
            t.what.clear();
        }
    }

    pub(crate) fn render_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        for image in self.retired.drain(..) {
            let _ = window.drop_image(image);
        }
        let now = Instant::now();
        let moving = self.springs.tick(now);
        if moving {
            window.request_animation_frame();
        }
        self.note_frame(now, moving);
        let g = Geo::from_array(self.springs.values());
        self.apply_shapes(g.shapes(), cx);
        if !moving && self.machine.state == State::Hidden && self.lift_at.is_none() && self.shown {
            self.settled_hidden(cx);
        }
        if self.focus_command
            && let Some(i) = self.inputs.clone()
        {
            self.focus_command = false;
            i.command.update(cx, |s, cx| s.focus(window, cx));
        }
        // The composer says what a message will do.
        if let (Page::Agent(id), Some(i)) = (&self.page, self.inputs.clone())
            && let Some(a) = self.snap.agents.iter().find(|a| a.id == *id)
        {
            let text = if self.question_of(*id).is_some() {
                "Type an answer…"
            } else if a.status == TaskStatus::Running {
                if a.steers { "Steer…" } else { "Queue for after this turn…" }
            } else {
                "Message…"
            };
            if self.composer_hint != text {
                self.composer_hint = text;
                i.message.update(cx, |s, cx| s.set_placeholder(text, window, cx));
            }
        }
        if self.hero.is_some_and(|h| h.at.elapsed().as_secs_f32() * 1000.0 >= tk::HERO_MS) {
            self.hero = None;
        }
        let state = self.machine.state;
        // Content is laid out at its final size, centered in the island and
        // clipped by it, so text never reflows while the shape springs. It
        // fades in over the last stretch of the opening.
        let target = Geo::from_array(self.springs.targets());
        let (tw, th) = (target.w, target.h);
        let open = (g.w / tw).min(g.h / th).clamp(0.0, 1.2);
        let reveal = if state == State::Hidden { 0.0 } else { ((open - tk::REVEAL_AT) / (1.0 - tk::REVEAL_AT)).clamp(0.0, 1.0) };
        let content = match state {
            State::Hidden => div().into_any_element(),
            State::Slim => self.render_slim(cx),
            State::Alert => self.render_alert(cx),
            State::Expanded => self.render_expanded(window, cx),
        };
        let menu = self.render_menu(cx);
        let composition = self.win.as_ref().is_some_and(|w| w.glass.is_some());
        let frosted = self.prefs.glass && self.transparency && (composition || self.backdrop.is_some());
        // The tint thickens over a bright backdrop, easing as it changes.
        let dt = now.duration_since(self.luma_frame).as_secs_f32();
        self.luma_frame = now;
        let goal = self.glass_luma;
        self.luma_shown += (goal - self.luma_shown) * (1.0 - (-dt / 0.25).exp());
        if (goal - self.luma_shown).abs() > 0.005 {
            window.request_animation_frame();
        } else {
            self.luma_shown = goal;
        }
        let dense = {
            let t = ((self.luma_shown - tk::LUMA_FROM) / (tk::LUMA_TO - tk::LUMA_FROM)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t) * tk::TINT_DENSE
        };
        let (top, bottom) = if frosted {
            (Hsla { a: tk::TINT_TOP.a + dense, ..tk::TINT_TOP }, Hsla { a: tk::TINT_BOTTOM.a + dense, ..tk::TINT_BOTTOM })
        } else {
            (tk::SOLID_TOP, tk::SOLID_BOTTOM)
        };
        let backdrop = self.backdrop.clone().filter(|_| frosted && !composition);
        let glow = self.glow();
        let needs = self.approval().is_some() && state != State::Expanded;
        let border = if needs { tk::AMBER.opacity(0.35 + 0.35 * breathe(self.t())) } else { tk::RIM };
        let grain = frosted.then(grain);
        let glass = move |d: Div, left: f32, y: f32, r: f32| {
            d.border_1()
                .border_color(border)
                .when_some(backdrop.clone(), |d, image| {
                    d.child(
                        img(image)
                            .absolute()
                            .left(px(-left - 1.0))
                            .top(px(-y - 1.0))
                            .w(px(tk::CANVAS_W))
                            .h(px(tk::CANVAS_H))
                            .object_fit(ObjectFit::Fill),
                    )
                })
                .child(div().absolute().top_0().left_0().size_full().bg(linear_gradient(180.0, linear_color_stop(top, 0.0), linear_color_stop(bottom, 1.0))))
                .when(frosted, |d| d.child(div().absolute().top_0().left_0().size_full().bg(tk::FROST)))
                .when_some(grain.clone(), |d, image| {
                    d.child(
                        img(image)
                            .absolute()
                            .left(px(-left - 1.0))
                            .top(px(-y - 1.0))
                            .w(px(tk::CANVAS_W))
                            .h(px(tk::CANVAS_H))
                            .object_fit(ObjectFit::Fill),
                    )
                })
                .when_some(glow, |d, c| {
                    d.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .bg(linear_gradient(180.0, linear_color_stop(c, 0.0), linear_color_stop(c.opacity(0.0), 0.7))),
                    )
                })
                // Light from above, and the pane's inner edge.
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(linear_gradient(180.0, linear_color_stop(tk::SHEEN, 0.0), linear_color_stop(tk::SHEEN.opacity(0.0), tk::SHEEN_TO))),
                )
                .child(div().absolute().top_0().left_0().size_full().rounded(px((r - 1.0).max(0.0))).border_1().border_color(tk::RIM_INNER))
                // The specular line along the top edge.
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(r * 0.7))
                        .right(px(r * 0.7))
                        .h(px(1.0))
                        .bg(linear_gradient(90.0, linear_color_stop(tk::SPECULAR.opacity(0.0), 0.0), linear_color_stop(tk::SPECULAR, 0.5))),
                )
        };
        let r = g.r.min(g.h / 2.0);
        let this = cx.entity().downgrade();
        let island = glass(div(), g.x(), g.y, r)
            .id("island")
            .absolute()
            .left(px(g.x()))
            .top(px(g.y))
            .w(px(g.w))
            .h(px(g.h))
            .rounded(px(r))
            .overflow_hidden()
            .text_color(tk::TEXT)
            .on_hover(move |hovered, _, cx| {
                let _ = this.update(cx, |n, cx| {
                    n.hovered = *hovered;
                    if !hovered {
                        n.pressed = false;
                    }
                    n.retarget(cx);
                });
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.pressed = true;
                    this.retarget(cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.pressed = false;
                    this.retarget(cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    if this.media.is_some() {
                        this.toggle_play(cx);
                    }
                }),
            )
            .when(state == State::Slim, |d| {
                d.cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.context_menu = !this.context_menu;
                            this.retarget(cx);
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                        if this.context_menu || matches!(ev, ClickEvent::Mouse(m) if m.up.button != MouseButton::Left) {
                            return;
                        }
                        if this.machine.open().is_some() {
                            this.transition(cx);
                        }
                    }))
            })
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let dy = match ev.delta {
                    ScrollDelta::Pixels(p) => f32::from(p.y),
                    ScrollDelta::Lines(l) => l.y * 20.0,
                };
                if dy.abs() < 1.0 {
                    return;
                }
                match this.machine.state {
                    State::Slim => {
                        this.primary = this.primary.wrapping_add(if dy < 0.0 { 1 } else { usize::MAX });
                        this.retarget(cx);
                        cx.notify();
                    }
                    State::Expanded if matches!(this.page, Page::Tab(Tab::Music)) => {
                        this.audio.nudge(if dy > 0.0 { 0.04 } else { -0.04 });
                        this.volume_shown_until = Some(Instant::now() + Duration::from_millis(1200));
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.add_paths(paths, cx)))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.border_color(tk::TEXT.opacity(0.6)).bg(tk::WASH))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left(px((g.w - tw) / 2.0))
                    .w(px(tw))
                    .h(px(th))
                    .opacity(reveal)
                    .child(content)
                    .children(menu),
            );
        let shapes = g.shapes();
        let mut bubbles: Vec<AnyElement> = Vec::new();
        for (i, s) in shapes.iter().skip(1).enumerate() {
            let b = glass(div(), s.x, s.y, s.r)
                .id(("bubble", i))
                .absolute()
                .left(px(s.x))
                .top(px(s.y))
                .size(px(s.w))
                .rounded_full()
                .overflow_hidden()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    let page = match this.slim_items().get(i + 1) {
                        Some(SlimItem::Music) => Page::Tab(Tab::Music),
                        Some(SlimItem::Timer) => Page::Tab(Tab::Timer),
                        Some(SlimItem::Note(_)) => Page::Tab(Tab::Notes),
                        _ => Page::Tab(Tab::Agents),
                    };
                    this.open_page(page, Some(window), cx);
                }))
                .child(div().absolute().top_0().left_0().size_full().opacity(reveal).child(self.render_bubble(i)));
            bubbles.push(b.into_any_element());
        }
        // A shared element in flight, drawn over everything.
        let hero = self.hero.map(|h| {
            let p = (h.at.elapsed().as_secs_f32() * 1000.0 / tk::HERO_MS).clamp(0.0, 1.0);
            let e = 1.0 - (1.0 - p).powi(3);
            let lerp = |a: f32, b: f32| a + (b - a) * e;
            let (x, y, s) = (lerp(h.from.0, h.to.0), lerp(h.from.1, h.to.1), lerp(h.from.2, h.to.2));
            window.request_animation_frame();
            let inner = match h.kind {
                HeroKind::Art => self.art(s, (s * 0.2).max(7.0)),
                HeroKind::Ring => progress_ring(1.0 - self.timer.progress(), s, 2.0 + 2.0 * e, tk::AMBER, div().into_any_element()),
            };
            div().absolute().left(px(x)).top(px(y)).size(px(s)).child(inner).into_any_element()
        });
        div()
            .id("notch-canvas")
            .w(px(tk::CANVAS_W))
            .h(px(tk::CANVAS_H))
            .relative()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| this.on_key(ev, window, cx)))
            .child(island)
            .children(bubbles)
            .children(hero)
            .into_any_element()
    }

    /// Keys while the island has the keyboard.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        let m = ev.keystroke.modifiers;
        let typing = self.inputs.as_ref().is_some_and(|i| {
            i.message.read(cx).focus_handle(cx).is_focused(window)
                || i.prompt.read(cx).focus_handle(cx).is_focused(window)
                || i.note.read(cx).focus_handle(cx).is_focused(window)
        });
        let in_command = self.inputs.as_ref().is_some_and(|i| i.command.read(cx).focus_handle(cx).is_focused(window));
        if m.control && key == "p" {
            let pinned = !self.machine.pinned;
            self.machine.set_pinned(pinned);
            self.toast(if pinned { "Kept open" } else { "Unpinned" });
            cx.notify();
            return;
        }
        if m.control && key == "o" {
            if let Page::Agent(id) = self.page {
                self.command(ToEngine::Open { task: Some(id) });
            }
            return;
        }
        // The open menu takes the arrows, Enter and Esc.
        if let Some(menu) = &mut self.menu {
            let actionable: Vec<usize> = menu.items.iter().enumerate().filter(|(_, i)| i.action.is_some()).map(|(i, _)| i).collect();
            let mut run = None;
            match key {
                "escape" => self.menu = None,
                "down" | "up" if !actionable.is_empty() => {
                    let at = menu.hovered.and_then(|h| actionable.iter().position(|i| *i == h));
                    let n = actionable.len();
                    let next = match (at, key) {
                        (None, "down") => 0,
                        (None, _) => n - 1,
                        (Some(a), "down") => (a + 1) % n,
                        (Some(a), _) => (a + n - 1) % n,
                    };
                    menu.hovered = Some(actionable[next]);
                }
                "enter" => run = menu.hovered.and_then(|h| menu.items.get(h)).and_then(|i| i.action.clone()),
                _ => {}
            }
            if let Some(action) = run {
                self.run_menu(action, window, cx);
            }
            self.retarget(cx);
            cx.notify();
            return;
        }
        let deep = !tab_of(&self.page).in_bar() || matches!(self.page, Page::Agent(_) | Page::NewAgent);
        match key {
            "escape" => {
                if self.picking {
                    self.picking = false;
                } else if self.popup().is_some() && self.machine.state == State::Alert {
                    self.dismiss_popup(cx);
                } else if !self.query.is_empty() {
                    self.query.clear();
                    if let Some(i) = self.inputs.clone() {
                        i.command.update(cx, |s, cx| s.set_value("", window, cx));
                    }
                    self.retarget(cx);
                } else if self.machine.state == State::Expanded && deep {
                    self.go_back(window, cx);
                } else if self.machine.close(self.rest(), self.alerting()).is_some() {
                    self.transition(cx);
                }
                cx.notify();
            }
            "backspace" if !typing && !in_command && self.machine.state == State::Expanded && deep => self.go_back(window, cx),
            "down" | "up" if !self.query.is_empty() => {
                let n = self.hits().len().max(1);
                self.hit = if key == "down" { (self.hit + 1) % n } else { (self.hit + n - 1) % n };
                cx.notify();
            }
            "down" | "up" | "left" | "right" if self.page == Page::Tab(Tab::Tray) && !typing && !in_command => {
                let n = self.tray_items().len();
                if n > 0 {
                    let cur = self.tray_selected.unwrap_or(0);
                    let step: isize = match key {
                        "down" => 2,
                        "up" => -2,
                        "right" => 1,
                        _ => -1,
                    };
                    self.tray_selected = Some(((cur as isize + step).rem_euclid(n as isize)) as usize);
                    cx.notify();
                }
            }
            "enter" if self.page == Page::Tab(Tab::Tray) && !typing && !in_command => {
                if let Some(ix) = self.tray_selected
                    && let Some(TrayItem::Clip(i)) = self.tray_items().get(ix).copied()
                    && let Some(c) = self.clips.get(i).cloned()
                {
                    let (t, im) = match c.body {
                        ClipBody::Text(t) => (Some(t), None),
                        ClipBody::Image(im) => (None, Some(im)),
                    };
                    self.paste_back(t, im, cx);
                }
            }
            "a" | "d" if !typing && !in_command && self.approval().is_some() => {
                if let Some((agent, req)) = self.approval() {
                    let (id, rid, safe) = (agent.id, req.request_id.clone(), req.safe && !req.question);
                    if key == "d" {
                        self.deny(id, rid, cx);
                    } else if safe {
                        self.decide(id, rid, PermissionDecision::AllowOnce, cx);
                    }
                    cx.notify();
                }
            }
            k if !typing && !in_command && k.len() == 1 && k.as_bytes()[0].is_ascii_digit() => {
                let n = (k.as_bytes()[0] - b'0') as usize;
                if let Some(t) = n.checked_sub(1).and_then(|i| self.prefs.tabs().get(i).copied()) {
                    self.open_page(Page::Tab(t), Some(window), cx);
                }
            }
            // Typing anywhere starts a search.
            k if !typing
                && !in_command
                && !m.control
                && !m.alt
                && self.machine.state == State::Expanded
                && k.len() == 1
                && k.as_bytes()[0].is_ascii_alphabetic() =>
            {
                let first = ev.keystroke.key_char.clone().unwrap_or_else(|| k.to_string());
                if let Some(i) = self.inputs.clone() {
                    let value = first.clone();
                    i.command.update(cx, |s, cx| {
                        s.set_value(value, window, cx);
                        s.focus(window, cx);
                    });
                }
                self.query = first;
                self.hit = 0;
                self.retarget(cx);
                cx.notify();
            }
            _ => {}
        }
    }
}

/// A pull request's checks as a ring: spinning while they run, red when
/// they failed.
fn pr_ring(checks: &str, size: f32) -> AnyElement {
    let running = checks == "pending";
    progress_ring(
        if running { -1.0 } else { 1.0 },
        size,
        2.0,
        if running { tk::BLUE } else { tk::RED },
        icon(icons::GIT_MERGE).size(px(12.0)).text_color(tk::TEXT).into_any_element(),
    )
}

/// Draws one screenshot mark: a red arrow or box, or a yellow wash.
fn paint_mark(window: &mut Window, tool: ShotTool, a: gpui::Point<gpui::Pixels>, b: gpui::Point<gpui::Pixels>) {
    match tool {
        ShotTool::Highlight => {
            let mut pb = gpui::PathBuilder::fill();
            pb.move_to(a);
            pb.line_to(point(b.x, a.y));
            pb.line_to(b);
            pb.line_to(point(a.x, b.y));
            pb.close();
            if let Ok(p) = pb.build() {
                window.paint_path(p, gpui::hsla(52.0 / 360.0, 1.0, 0.55, 0.35));
            }
        }
        ShotTool::Box => {
            let mut pb = gpui::PathBuilder::stroke(px(2.5));
            pb.move_to(a);
            pb.line_to(point(b.x, a.y));
            pb.line_to(b);
            pb.line_to(point(a.x, b.y));
            pb.line_to(a);
            if let Ok(p) = pb.build() {
                window.paint_path(p, tk::RED);
            }
        }
        ShotTool::Arrow => {
            let ang = f32::from(b.y - a.y).atan2(f32::from(b.x - a.x));
            let mut pb = gpui::PathBuilder::stroke(px(2.5));
            pb.move_to(a);
            pb.line_to(b);
            for side in [-0.5f32, 0.5] {
                let t = ang + std::f32::consts::PI + side;
                pb.move_to(b);
                pb.line_to(point(b.x + px(12.0 * t.cos()), b.y + px(12.0 * t.sin())));
            }
            if let Ok(p) = pb.build() {
                window.paint_path(p, tk::RED);
            }
        }
    }
}

/// A canvas-sized grain texture, made once: sparse light and dark specks at
/// a low alpha (premultiplied), laid over the blur so it reads as frosted.
fn grain() -> std::sync::Arc<gpui::RenderImage> {
    static GRAIN: std::sync::OnceLock<std::sync::Arc<gpui::RenderImage>> = std::sync::OnceLock::new();
    GRAIN
        .get_or_init(|| {
            let (w, h) = (tk::CANVAS_W as u32, tk::CANVAS_H as u32);
            let mut seed: u32 = 0x9e37_79b9;
            let img = image::RgbaImage::from_fn(w, h, |_, _| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let a = (seed & 0xff) as u32 * tk::GRAIN as u32 / 255;
                let a = a as u8;
                // Half the specks lighten, half darken.
                if seed & 0x100 == 0 { image::Rgba([a, a, a, a]) } else { image::Rgba([0, 0, 0, a]) }
            });
            std::sync::Arc::new(gpui::RenderImage::new([image::Frame::new(img)]))
        })
        .clone()
}
