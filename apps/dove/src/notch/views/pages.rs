//! The expanded island's pages (docs/NOTCH-V2.md §11.4): Home, Agents, the
//! agent view, New agent, Music, Tray, Notes, Timer, Notifications and Dev
//! servers. Each shows what you'd use most of the time; the rest is on
//! hover, in its `…` menu, or in the command bar.

use super::*;

impl Notch {
    pub(crate) fn page_hits(&self, cx: &mut Context<Self>) -> AnyElement {
        let hits = self.hits();
        let mut col = div().id("hits").size_full().overflow_y_scroll().flex().flex_col().gap(px(2.0)).pt(px(4.0));
        for (i, (hit, label, kind)) in hits.into_iter().enumerate() {
            let on = i == self.hit;
            let glyph = match hit {
                Hit::Agent(_) | Hit::Stop(_) | Hit::OpenInDove(_) => icons::BOT,
                Hit::Note(_) | Hit::NewNote(_) => icons::NOTE_TEXT,
                Hit::Clip(_) | Hit::ClearClips => icons::COPY,
                Hit::Tab(t) => glyph(t),
                Hit::NewAgent(_) => icons::SPARKLE_AGENT,
                Hit::Ask(_) => icons::CHAT_ROUND_LINE,
                Hit::Timer(_) | Hit::LinkTimer(_) => icons::TIMER,
                Hit::Capture => icons::SCREEN_CAPTURE,
                Hit::Focus => icons::MOON,
                Hit::Pin => icons::PIN,
                Hit::Shuffle => icons::MEDIA_SHUFFLE,
                Hit::RunAutomation(_) => icons::BOLT,
                Hit::Merge(..) => icons::GIT_MERGE,
            };
            col = col.child(
                div()
                    .id(("hit", i))
                    .h(px(40.0))
                    .px(px(10.0))
                    .rounded(px(tk::R_ROW))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .cursor_pointer()
                    .when(on, |d| d.bg(tk::WASH_HOVER))
                    .hover(|s| s.bg(tk::WASH))
                    .child(icon(glyph).size(px(tk::ICON_ROW)).text_color(if on { tk::TEXT } else { tk::DIM }))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(tk::T13)).text_color(tk::TEXT).child(label))
                    .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(kind))
                    .when(on, |d| d.child(icon(icons::RETURN).size(px(13.0)).text_color(tk::FAINT)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.run_hit(Some(i), window, cx))),
            );
        }
        col.into_any_element()
    }

    /// An agent's row: its mark in a ring, the title and a live ticker; the
    /// status at rest, one `…` on hover. Files dropped on it go to it.
    pub(crate) fn agent_row(&self, a: &Agent, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let id = a.id;
        let (label, color) = status_of(a);
        let ticker =
            a.activity.clone().filter(|s| !s.is_empty()).or_else(|| (!a.steps.is_empty()).then(|| a.steps.clone()));
        let now = dove_core::store::now_ms();
        let time = a.elapsed_secs(now).filter(|s| *s > 0).map(elapsed);
        let running = a.status == TaskStatus::Running && a.live;
        let group = SharedString::from(format!("agent-row-{ix}"));
        div()
            .id(("agent", ix))
            .group(group.clone())
            .h(px(tk::ROW_H))
            .px(px(8.0))
            .rounded(px(tk::R_ROW))
            .flex()
            .items_center()
            .gap(px(10.0))
            .cursor_pointer()
            .hover(|s| s.bg(tk::WASH))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(tk::BLUE.opacity(0.16)))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                cx.stop_propagation();
                this.drop_on_agent(id, paths, cx);
            }))
            .on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| this.open_page(Page::Agent(id), Some(window), cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    let items = this.agent_menu(id, running);
                    this.open_menu(ev.position, items, cx);
                }),
            )
            .child(avatar(a.kind, 28.0, agent_progress(a)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(tk::T13))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(tk::TEXT)
                            .truncate()
                            .child(a.title.clone()),
                    )
                    .child(match ticker {
                        Some(t) => div()
                            .font_family(tk::MONO)
                            .text_size(px(tk::T11))
                            .text_color(tk::FAINT)
                            .truncate()
                            .child(t)
                            .into_any_element(),
                        None => div()
                            .text_size(px(tk::T11))
                            .text_color(tk::FAINT)
                            .truncate()
                            .child(a.project.clone())
                            .into_any_element(),
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .flex_none()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.0))
                                    .child(self.dot(color, 6.0, matches!(label, "Working" | "Needs you")))
                                    .child(div().text_size(px(tk::T11)).text_color(color).child(label)),
                            )
                            .child(div().text_size(px(tk::T11)).text_color(tk::GHOST).child(match (time, a.cost) {
                                (Some(t), Some(c)) => format!("{t} · ${c:.2}"),
                                (Some(t), None) => t,
                                (None, Some(c)) => format!("${c:.2}"),
                                (None, None) => String::new(),
                            })),
                    )
                    .child(div().invisible().group_hover(group, |s| s.visible()).child(
                        ibtn(("row-more", ix), icons::MORE_HORIZONTAL, 26.0, tk::DIM, "More").on_click(cx.listener(
                            move |this, ev: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                let items = this.agent_menu(id, running);
                                this.open_menu(ev.position(), items, cx);
                            },
                        )),
                    )),
            )
            .into_any_element()
    }

    // ----- home -----------------------------------------------------------------------

    /// Home's cards in attention order (the first three show).
    pub(crate) fn home_cards(&self) -> Vec<HomeCard> {
        let mut out = Vec::new();
        if !self.connected() && self.snap.agents.is_empty() {
            out.push(HomeCard::Offline);
        }
        if self.approval().is_some() {
            out.push(HomeCard::Approval);
        }
        if self.snap.agents.iter().any(|a| a.active() && !a.needs_you()) {
            out.push(HomeCard::Working);
        }
        if self.media.is_some() {
            out.push(HomeCard::Music);
        }
        if self.timer.active() {
            out.push(HomeCard::Timer);
        }
        if !self.ports.is_empty() {
            out.push(HomeCard::Dev);
        }
        if self.snap.prs.iter().any(|p| p.state == "open") {
            out.push(HomeCard::Prs);
        }
        let now = dove_core::store::now_ms();
        if self
            .snap
            .automations
            .iter()
            .any(|a| a.next_ms.is_some_and(|n| n < now + 86_400_000) || a.last_ms.is_some_and(|l| now - l < 3_600_000))
        {
            out.push(HomeCard::Automations);
        }
        if !self.notes.is_empty() {
            out.push(HomeCard::Note);
        }
        if !self.clips.is_empty() {
            out.push(HomeCard::Clips);
        }
        out
    }

    pub(crate) fn page_home(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let key = self.key();
        let cards = self.home_cards();
        if cards.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.0))
                .child(icon(icons::DOVE_LOGO).size(px(26.0)).text_color(tk::FAINT))
                .child(div().text_size(px(tk::T13)).text_color(tk::DIM).child("All quiet"))
                .child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(primary("empty-agent", "Start an agent").on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| this.open_page(Page::NewAgent, Some(window), cx),
                        )))
                        .child(pill("empty-timer", "Timer", false).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| this.open_page(Page::Tab(Tab::Timer), Some(window), cx),
                        )))
                        .child(pill("empty-note", "New note", false).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                this.selected_note = None;
                                this.open_page(Page::Tab(Tab::Notes), Some(window), cx);
                            },
                        ))),
                )
                .into_any_element();
        }
        let mut col = div().id("home").size_full().overflow_y_scroll().flex().flex_col().gap(px(tk::GAP));
        for (i, c) in cards.iter().take(3).enumerate() {
            let el = self.home_card(*c, cx);
            col = col.child(rise(el, (key, i), i));
        }
        if cards.len() > 3 {
            let mut more = div().flex().items_center().gap(px(6.0)).child(section("Also"));
            for (i, c) in cards.iter().skip(3).enumerate() {
                let c = *c;
                more = more.child(pill(("more", i), c.label(), false).h(px(24.0)).px(px(10.0)).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        let page = c.page(this);
                        this.open_page(page, Some(window), cx);
                    },
                )));
            }
            col = col.child(rise(more, (key, 3), 3));
        }
        col.into_any_element()
    }

    fn home_card(&mut self, c: HomeCard, cx: &mut Context<Self>) -> AnyElement {
        match c {
            HomeCard::Offline => card()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(icon(icons::DOVE_LOGO).size(px(18.0)).text_color(tk::DIM))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(tk::T13)).text_color(tk::TEXT).child("Dove isn't running"))
                        .child(
                            div()
                                .text_size(px(tk::T11))
                                .text_color(tk::FAINT)
                                .child("Music, the tray, notes and the timer still work."),
                        ),
                )
                .child(link("open-dove", "Open Dove").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.command(ToEngine::Open { task: None });
                    this.toast("Opening Dove…");
                    cx.notify();
                })))
                .into_any_element(),
            HomeCard::Approval => {
                let Some((agent, req)) = self.approval() else { return div().into_any_element() };
                let (id, req, kind, title) = (agent.id, req.clone(), agent.kind, agent.title.clone());
                card()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(mark(kind, 15.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(tk::T13))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(tk::TEXT)
                                    .child(title),
                            )
                            .child(self.dot(tk::AMBER, 6.0, true)),
                    )
                    .child(
                        div()
                            .text_size(px(tk::T12))
                            .text_color(tk::DIM)
                            .line_clamp(2)
                            .child(format!("{} · {}", req.tool, req.summary)),
                    )
                    .when_some(req.preview.clone(), |d, p| d.child(preview(&p, 3)))
                    .child(self.decision_row(id, &req, "home-decide", cx))
                    .into_any_element()
            }
            HomeCard::Working => {
                let working: Vec<Agent> =
                    self.snap.agents.iter().filter(|a| a.active() && !a.needs_you()).take(3).cloned().collect();
                let mut c = div().flex().flex_col().gap(px(2.0));
                for (i, a) in working.iter().enumerate() {
                    c = c.child(self.agent_row(a, 100 + i, cx));
                }
                c.into_any_element()
            }
            HomeCard::Music => {
                let Some(m) = self.media.clone() else { return div().into_any_element() };
                card()
                    .id("home-music")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_page(Page::Tab(Tab::Music), Some(window), cx)
                    }))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(self.art(44.0, 11.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .child(
                                        div()
                                            .text_size(px(tk::T13))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(tk::TEXT)
                                            .truncate()
                                            .child(m.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(tk::T11))
                                            .text_color(tk::FAINT)
                                            .truncate()
                                            .child(m.artist.clone()),
                                    ),
                            )
                            .child(bar(m.progress(), tk::TEXT.opacity(0.8), 3.0)),
                    )
                    .child(self.eq(14.0))
                    .child(self.play_button("home-play", 30.0, cx))
                    .into_any_element()
            }
            HomeCard::Timer => card()
                .id("home-timer")
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_page(Page::Tab(Tab::Timer), Some(window), cx)
                }))
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(progress_ring(
                    self.timer.progress(),
                    40.0,
                    3.0,
                    tk::AMBER,
                    icon(icons::TIMER).size(px(15.0)).text_color(tk::AMBER).into_any_element(),
                ))
                .child(roll(&mins_secs(self.timer.remaining()), "home-timer", tk::T20, tk::TEXT, FontWeight::SEMIBOLD))
                .child(div().flex_1())
                .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(if self.timer.running() {
                    "Running"
                } else {
                    "Paused"
                }))
                .into_any_element(),
            HomeCard::Dev => {
                let mut c = card().flex().flex_col().gap(px(4.0));
                for (i, p) in self.ports.iter().take(3).enumerate() {
                    let url = format!("http://localhost:{}", p.port);
                    c = c.child(
                        div()
                            .id(("home-port", i))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .cursor_pointer()
                            .rounded(px(8.0))
                            .hover(|s| s.bg(tk::WASH))
                            .child(self.dot(tk::GREEN, 6.0, false))
                            .child(
                                div()
                                    .font_family(tk::MONO)
                                    .text_size(px(tk::T12))
                                    .text_color(tk::TEXT)
                                    .child(format!("localhost:{}", p.port)),
                            )
                            .child(div().flex_1().text_size(px(tk::T11)).text_color(tk::FAINT).child(p.process.clone()))
                            .child(icon(icons::ARROW_UP_RIGHT).size(px(13.0)).text_color(tk::FAINT))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    );
                }
                c.into_any_element()
            }
            HomeCard::Prs => {
                let mut c = card().flex().flex_col().gap(px(6.0));
                let prs: Vec<_> = self.snap.prs.iter().filter(|p| p.state == "open").take(3).cloned().collect();
                for (i, pr) in prs.into_iter().enumerate() {
                    let (color, label) = match pr.checks.as_str() {
                        "passing" => (tk::GREEN, "Checks passed"),
                        "failing" => (tk::RED, "Checks failed"),
                        "pending" => (tk::BLUE, "Checks running"),
                        _ => (tk::FAINT, "No checks"),
                    };
                    let (task, number, url) = (pr.task, pr.number, pr.url.clone());
                    let mergeable = pr.mergeable && pr.checks == "passing";
                    c = c.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(mark(pr.kind, 14.0))
                            .child(
                                div()
                                    .id(("home-pr", i))
                                    .flex_1()
                                    .min_w_0()
                                    .cursor_pointer()
                                    .on_click(move |_, _, cx| cx.open_url(&url))
                                    .child(
                                        div()
                                            .text_size(px(tk::T12))
                                            .text_color(tk::TEXT)
                                            .truncate()
                                            .child(format!("#{} {}", pr.number, pr.title)),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(5.0))
                                            .text_size(px(tk::T11))
                                            .text_color(color)
                                            .child(self.dot(color, 5.0, pr.checks == "pending"))
                                            .child(label),
                                    ),
                            )
                            .when(mergeable, |d| {
                                d.child(link(("home-merge", i), "Merge").on_click(cx.listener(
                                    move |this, _: &ClickEvent, _, cx| {
                                        this.command(ToEngine::Merge { task, number });
                                        this.toast("Merging…");
                                        cx.notify();
                                    },
                                )))
                            }),
                    );
                }
                c.into_any_element()
            }
            HomeCard::Automations => {
                let mut c = card().flex().flex_col().gap(px(6.0));
                let mut list: Vec<_> = self.snap.automations.iter().filter(|a| a.enabled).cloned().collect();
                list.sort_by_key(|a| a.next_ms.unwrap_or(i64::MAX));
                for (i, a) in list.into_iter().take(2).enumerate() {
                    let line = match (a.next_ms, a.last_status) {
                        (Some(n), _) => format!("Next {}", until(n)),
                        (None, Some(TaskStatus::Failed)) => "Last run failed".to_string(),
                        (None, Some(_)) => format!("Ran {}", a.last_ms.map(ago).unwrap_or_default()),
                        _ => "On a trigger".into(),
                    };
                    let aid = a.id.clone();
                    c = c.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(icon(icons::BOLT).size(px(15.0)).text_color(tk::DIM))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(tk::T12))
                                            .text_color(tk::TEXT)
                                            .truncate()
                                            .child(a.name.clone()),
                                    )
                                    .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(line)),
                            )
                            .child(link(("run-auto", i), "Run now").on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.command(ToEngine::RunAutomation { id: aid.clone() });
                                    this.toast("Running");
                                    cx.notify();
                                },
                            ))),
                    );
                }
                c.into_any_element()
            }
            HomeCard::Note => {
                let Some(n) = self.notes.iter().find(|n| n.pinned).or(self.notes.first()).cloned() else {
                    return div().into_any_element();
                };
                let id = n.id;
                card()
                    .id("home-note")
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.selected_note = Some(id);
                        this.open_page(Page::Tab(Tab::Notes), Some(window), cx);
                    }))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .when(n.pinned, |d| d.child(icon(icons::PIN).size(px(12.0)).text_color(tk::AMBER)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(tk::T13))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(tk::TEXT)
                                    .truncate()
                                    .child(n.title()),
                            )
                            .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(match n.checklist() {
                                Some((d, t)) => format!("{d}/{t}"),
                                None => String::new(),
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(tk::T12))
                            .text_color(tk::DIM)
                            .line_clamp(2)
                            .child(n.text.lines().skip(1).collect::<Vec<_>>().join(" ")),
                    )
                    .into_any_element()
            }
            HomeCard::Clips => {
                let mut c = card().flex().flex_col().gap(px(2.0));
                for (i, clip) in self.clips.iter().take(3).enumerate() {
                    let text = clip
                        .as_text()
                        .map(|t| t.trim().lines().next().unwrap_or("").to_string())
                        .unwrap_or_else(|| "Image".into());
                    let copy = clip.clone();
                    c = c.child(
                        div()
                            .id(("home-clip", i))
                            .px(px(6.0))
                            .py(px(3.0))
                            .rounded(px(8.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(tk::WASH))
                            .child(icon(icons::COPY).size(px(13.0)).text_color(tk::FAINT))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(tk::T12))
                                    .text_color(tk::DIM)
                                    .child(text),
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.write_to_clipboard(match &copy.body {
                                    ClipBody::Text(t) => ClipboardItem::new_string(t.clone()),
                                    ClipBody::Image(im) => ClipboardItem::new_image(im),
                                });
                                this.toast("Copied");
                                cx.notify();
                            })),
                    );
                }
                c.into_any_element()
            }
        }
    }

    // ----- agents -----------------------------------------------------------------------

    pub(crate) fn page_agents(&self, cx: &mut Context<Self>) -> AnyElement {
        let new = primary("agents-new", "New agent")
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_page(Page::NewAgent, Some(window), cx)));
        if self.snap.agents.is_empty() && self.snap.flock.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .child(div().text_size(px(tk::T13)).text_color(tk::DIM).child("No agents yet"))
                .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child("Or type “new …” in the search"))
                .child(new)
                .into_any_element();
        }
        let key = self.key();
        let mut col = div()
            .id("agents")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(div().h(px(32.0)).flex().items_center().px(px(4.0)).child(div().flex_1()).child(new));
        let mut index = 0;
        let groups: [(&str, Vec<&Agent>); 3] = [
            ("Needs you", self.snap.agents.iter().filter(|a| a.needs_you()).collect()),
            (
                "Working",
                self.snap.agents.iter().filter(|a| a.status == TaskStatus::Running && !a.needs_you()).collect(),
            ),
            ("Recent", self.snap.agents.iter().filter(|a| !a.active()).collect()),
        ];
        for (label, agents) in groups {
            if agents.is_empty() {
                continue;
            }
            col = col.child(div().pt(px(6.0)).pb(px(2.0)).px(px(8.0)).child(section(label)));
            for a in agents {
                col = col.child(rise(self.agent_row(a, index, cx), (key, index), index));
                index += 1;
            }
        }
        if !self.snap.flock.is_empty() {
            col = col.child(div().pt(px(6.0)).pb(px(2.0)).px(px(8.0)).child(section("Flock")));
            for (i, f) in self.snap.flock.iter().enumerate() {
                let (label, color) = if f.needs_input {
                    ("Needs you", tk::AMBER)
                } else if f.working {
                    ("Working", tk::BLUE)
                } else {
                    ("Idle", tk::FAINT)
                };
                col = col.child(
                    div()
                        .id(("flock", i))
                        .h(px(tk::ROW_H))
                        .px(px(8.0))
                        .rounded(px(tk::R_ROW))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(tk::WASH))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, _| this.command(ToEngine::OpenFlock)))
                        .child(avatar(f.kind, 28.0, (f.working || f.needs_input).then_some((-1.0, color))))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(tk::T13))
                                .text_color(tk::TEXT)
                                .child(f.place.clone()),
                        )
                        .child(self.dot(color, 6.0, f.working || f.needs_input))
                        .child(div().text_size(px(tk::T11)).text_color(color).child(label)),
                );
            }
        }
        // Plan usage, only once it's worth a glance.
        let busy: Vec<_> = self.snap.limits.iter().filter(|m| m.used >= 50.0).collect();
        if !busy.is_empty() {
            let text = busy
                .iter()
                .take(2)
                .map(|m| {
                    let reset = m.resets_at.map(|s| format!(", resets {}", until(s * 1000))).unwrap_or_default();
                    format!("{} {} {:.0}%{reset}", m.kind.label(), m.label, m.used)
                })
                .collect::<Vec<_>>()
                .join(" · ");
            let hot = busy.iter().any(|m| m.used >= 80.0);
            col = col.child(
                div()
                    .mt(px(6.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(tk::T11))
                    .text_color(if hot { tk::AMBER } else { tk::FAINT })
                    .child(icon(icons::CHART).size(px(13.0)).text_color(if hot { tk::AMBER } else { tk::FAINT }))
                    .child(div().truncate().child(text)),
            );
        }
        col.into_any_element()
    }

    // ----- the agent view ----------------------------------------------------------------

    /// The agent's recent turns: the engine's while the island watches it,
    /// else its last reply.
    pub(crate) fn turns_of(&self, id: TaskId) -> Vec<TurnView> {
        if let Some(f) = self.snap.focus.as_ref().filter(|f| f.id == id)
            && !f.turns.is_empty()
        {
            return f.turns.clone();
        }
        let reply = if self.reply.trim().is_empty() {
            self.snap.agents.iter().find(|a| a.id == id).map(|a| a.reply.clone()).unwrap_or_default()
        } else {
            self.reply.clone()
        };
        if reply.trim().is_empty() { Vec::new() } else { vec![TurnView { reply, done: true, ..Default::default() }] }
    }

    pub(crate) fn page_agent(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(a) = self.snap.agents.iter().find(|a| a.id == id).cloned() else { return div().into_any_element() };
        let (label, color) = status_of(&a);
        let running = a.status == TaskStatus::Running && a.live;
        let stopping = self.stopping.is_some_and(|(t, _)| t == id);
        let turns = self.turns_of(id);
        if self.stick {
            self.reply_scroll.scroll_to_bottom();
        }
        // The title row: the mark in its ring, the title (opens in Dove), one
        // status word, and Stop while it works.
        let meta: Vec<String> = [
            a.branch.clone(),
            a.worktree.then(|| "worktree".to_string()),
            a.elapsed_secs(dove_core::store::now_ms()).filter(|s| *s > 0).map(elapsed),
            a.cost.map(|c| format!("${c:.2}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let title = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(avatar(a.kind, 28.0, agent_progress(&a)))
            .child(
                div()
                    .id("agent-title")
                    .flex_1()
                    .min_w_0()
                    .cursor_pointer()
                    .tooltip(tip("Open in Dove · Ctrl+O"))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, _| this.command(ToEngine::Open { task: Some(id) })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(tk::T13))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(tk::TEXT)
                                    .child(a.title.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .text_size(px(tk::T11))
                                    .text_color(color)
                                    .child(self.dot(color, 6.0, running || label == "Needs you"))
                                    .child(label),
                            ),
                    )
                    .when(!meta.is_empty(), |d| {
                        d.child(
                            div()
                                .font_family(tk::MONO)
                                .text_size(px(tk::T11))
                                .text_color(tk::FAINT)
                                .truncate()
                                .child(meta.join(" · ")),
                        )
                    }),
            )
            .when(running && !stopping, |d| {
                d.child(
                    ibtn("agent-stop", icons::STOP, 30.0, tk::RED, "Stop")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.stop_agent(id, cx))),
                )
            });

        // The mini transcript.
        let mut log = div().flex().flex_col().gap(px(10.0)).pb(px(4.0));
        if turns.len() > 1 || turns.first().is_some_and(|t| !t.user.is_empty()) {
            log = log.child(div().flex().justify_center().child(link("earlier", "Earlier in Dove").on_click(
                cx.listener(move |this, _: &ClickEvent, _, _| this.command(ToEngine::Open { task: Some(id) })),
            )));
        }
        for (ti, t) in turns.iter().enumerate() {
            if !t.user.trim().is_empty() {
                log = log.child(bubble(&t.user, None));
            }
            if !t.steps.is_empty() {
                let open = self.unfolded.contains(&ti);
                let failed = t.failed > 0;
                log = log.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .id(("steps", ti))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .cursor_pointer()
                                .text_size(px(tk::T11))
                                .text_color(if failed { tk::RED.opacity(0.85) } else { tk::FAINT })
                                .hover(|s| s.text_color(tk::DIM))
                                .child(
                                    icon(if open { icons::ALT_ARROW_DOWN } else { icons::CHECKLIST })
                                        .size(px(13.0))
                                        .text_color(tk::FAINT),
                                )
                                .child(div().truncate().child(t.steps.clone()))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if !this.unfolded.remove(&ti) {
                                        this.unfolded.insert(ti);
                                    }
                                    this.retarget(cx);
                                    cx.notify();
                                })),
                        )
                        .when(open, |d| {
                            d.children(t.files.iter().enumerate().map(|(fi, f)| {
                                let name = f.rsplit(['/', '\\']).next().unwrap_or(f).to_string();
                                div()
                                    .id(("step-file", ti * 100 + fi))
                                    .pl(px(19.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .cursor_pointer()
                                    .font_family(tk::MONO)
                                    .text_size(px(tk::T11))
                                    .text_color(tk::DIM)
                                    .hover(|s| s.text_color(tk::TEXT))
                                    .tooltip(tip("See the change in Dove"))
                                    .child(icon(icons::FILE_CODE).size(px(12.0)).text_color(tk::FAINT))
                                    .child(div().truncate().child(name))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, _| {
                                        this.command(ToEngine::Open { task: Some(id) })
                                    }))
                            }))
                        }),
                );
            }
            if !t.reply.trim().is_empty() {
                log = log.child(markdown(&t.reply, &format!("t{ti}")));
            }
        }
        for m in self.sent.iter().filter(|m| m.task == id) {
            let note = if !self.connected() {
                "waiting for Dove"
            } else if running && !a.steers {
                "queued for after this turn"
            } else {
                "sending"
            };
            log = log.child(bubble(&m.text, Some(note)));
        }
        if running && let Some(act) = a.activity.clone().filter(|s| !s.is_empty()) {
            log = log.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(tk::T11))
                    .text_color(tk::FAINT)
                    .child(self.dot(tk::BLUE, 5.0, true))
                    .child(div().font_family(tk::MONO).truncate().child(act)),
            );
        }
        if turns.is_empty() && self.sent.iter().all(|m| m.task != id) {
            log = log.child(div().text_size(px(tk::T12)).text_color(tk::FAINT).child("No messages yet."));
        }
        let stick = self.stick;
        let transcript = div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("agent-log")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.reply_scroll)
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                        let dy = match ev.delta {
                            ScrollDelta::Pixels(p) => f32::from(p.y),
                            ScrollDelta::Lines(l) => l.y * 20.0,
                        };
                        if dy > 0.5 {
                            this.stick = false;
                        } else {
                            let (off, max) = (this.reply_scroll.offset().y, this.reply_scroll.max_offset().y);
                            if (f32::from(max) + f32::from(off)).abs() < 8.0 {
                                this.stick = true;
                            }
                        }
                        cx.notify();
                    }))
                    .child(log),
            )
            .when(!stick, |d| {
                d.child(div().absolute().bottom(px(6.0)).left_0().right_0().flex().justify_center().child(
                    pill("jump", "Jump to latest", true).h(px(24.0)).shadow_md().on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.stick = true;
                            cx.notify();
                        },
                    )),
                ))
            });

        // What it waits on: one approval or question at a time, pinned.
        let pending: Vec<Pending> =
            a.pending.iter().filter(|p| !self.dismissed.contains(&p.request_id)).cloned().collect();
        let waiting = pending.first().cloned().map(|req| {
            let count = pending.len();
            if req.question && !req.questions.is_empty() {
                self.question_card(id, &req, cx)
            } else {
                card()
                    .flex()
                    .flex_col()
                    .gap(px(7.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(self.dot(tk::AMBER, 6.0, true))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(tk::T12))
                                    .text_color(tk::TEXT)
                                    .child(format!("{} · {}", req.tool, req.summary)),
                            )
                            .when(count > 1, |d| {
                                d.child(
                                    div().text_size(px(tk::T11)).text_color(tk::FAINT).child(format!("1 of {count}")),
                                )
                            }),
                    )
                    .when_some(req.preview.clone(), |d, p| d.child(preview(&p, 6)))
                    .child(self.decision_row(id, &req, "agent-decide", cx))
                    .into_any_element()
            }
        });

        // The composer.
        let composer_focused =
            self.inputs.as_ref().is_some_and(|i| i.message.read(cx).focus_handle(cx).is_focused(window));
        let empty = self.inputs.as_ref().is_none_or(|i| i.message.read(cx).value().trim().is_empty());
        let quick = (composer_focused && empty && waiting.is_none() && !self.picking).then(|| {
            let mut chips = div().flex().gap(px(6.0)).flex_wrap();
            for (i, q) in ["Continue", "Run the tests", "Explain what you did"].iter().enumerate() {
                let q = q.to_string();
                chips = chips.child(pill(("quick", i), q.clone(), false).h(px(24.0)).px(px(10.0)).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.sent.push(super::super::Sent { task: id, text: q.clone(), at: Instant::now() });
                        this.command(ToEngine::Send { task: id, text: q.clone() });
                        this.stick = true;
                        this.retarget(cx);
                        cx.notify();
                    }),
                ));
            }
            chips
        });
        let picker = self.picking.then(|| {
            let mut list = card().p(px(6.0)).flex().flex_col();
            if self.shelf.is_empty() {
                list = list.child(
                    div()
                        .p(px(6.0))
                        .text_size(px(tk::T12))
                        .text_color(tk::FAINT)
                        .child("Drop files on the island to attach them"),
                );
            }
            for (i, p) in self.shelf.iter().take(5).enumerate() {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let path = p.clone();
                list = list.child(
                    div()
                        .id(("pick", i))
                        .h(px(30.0))
                        .px(px(8.0))
                        .rounded(px(10.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(tk::WASH_HOVER))
                        .child(
                            icon(if p.is_dir() { icons::FOLDER } else { icons::DOCUMENT })
                                .size(px(14.0))
                                .text_color(tk::DIM),
                        )
                        .child(
                            div().flex_1().min_w_0().truncate().text_size(px(tk::T12)).text_color(tk::TEXT).child(name),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.attach(path.clone(), cx))),
                );
            }
            list
        });
        let chips = (!self.attachments.is_empty()).then(|| {
            div().flex().flex_wrap().gap(px(6.0)).children(self.attachments.iter().enumerate().map(|(i, p)| {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                div()
                    .h(px(24.0))
                    .pl(px(8.0))
                    .pr(px(4.0))
                    .rounded_full()
                    .bg(tk::WASH)
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .text_size(px(tk::T11))
                    .text_color(tk::TEXT)
                    .child(format!("@{name}"))
                    .child(
                        div()
                            .id(("unattach", i))
                            .size(px(16.0))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(tk::WASH_HOVER))
                            .child(icon(icons::CLOSE).size(px(10.0)).text_color(tk::DIM))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                if i < this.attachments.len() {
                                    this.attachments.remove(i);
                                }
                                cx.notify();
                            })),
                    )
            }))
        });
        let message = self.inputs.as_ref().map(|i| i.message.clone());
        let dictating = self.dictation_base.is_some();
        let ready = !empty || !self.attachments.is_empty();
        let composer = div()
            .min_h(px(40.0))
            .pl(px(14.0))
            .pr(px(5.0))
            .py(px(5.0))
            .rounded(px(20.0))
            .bg(tk::WASH)
            .flex()
            .items_end()
            .gap(px(4.0))
            .text_size(px(tk::T12))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _, cx| this.take_focus(cx)))
            .children(message.map(|m| div().flex_1().min_w_0().py(px(5.0)).child(Textarea::new(&m))))
            .child(
                div()
                    .id("mic")
                    .size(px(30.0))
                    .flex_none()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(dictating, |d| d.bg(tk::RED.opacity(0.2)))
                    .hover(|s| s.bg(tk::WASH_HOVER))
                    .tooltip(tip("Hold to talk"))
                    .child(icon(icons::MICROPHONE).size(px(tk::ICON_ROW)).text_color(if dictating {
                        tk::RED
                    } else {
                        tk::DIM
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.dictate(true, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| this.dictate(false, cx)),
                    ),
            )
            .child(
                div()
                    .id("agent-send")
                    .size(px(30.0))
                    .flex_none()
                    .rounded_full()
                    .bg(if ready { tk::WHITE } else { tk::WASH_HOVER })
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.85))
                    .tooltip(tip("Send · Enter"))
                    .child(icon(icons::SEND_UP).size(px(15.0)).text_color(if ready { tk::INK } else { tk::DIM }))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.send_message(window, cx))),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(title)
            .child(transcript)
            .children(waiting)
            .children(quick)
            .children(picker)
            .children(chips)
            .child(composer)
            .into_any_element()
    }

    /// A question asked in the agent view: its options as pills (several
    /// for a multi-select, with Next), or an answer typed in the composer.
    fn question_card(&self, id: TaskId, req: &Pending, cx: &mut Context<Self>) -> AnyElement {
        let a = self.answers.get(&req.request_id).cloned().unwrap_or_default();
        let page = a.page.min(req.questions.len().saturating_sub(1));
        let Some(q) = req.questions.get(page).cloned() else { return div().into_any_element() };
        let picked = a.picks.get(page).cloned().unwrap_or_default();
        let mut opts = div().flex().flex_wrap().gap(px(6.0));
        for (i, o) in q.options.iter().enumerate() {
            let (rid, o2) = (req.request_id.clone(), o.clone());
            let on = picked.contains(o);
            opts = opts.child(pill(("opt", i), o.clone(), on).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| this.answer_pick(id, rid.clone(), o2.clone(), false, cx),
            )));
        }
        let rid = req.request_id.clone();
        card()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(self.dot(tk::AMBER, 6.0, true))
                    .child(
                        div().flex_1().min_w_0().text_size(px(tk::T12)).text_color(tk::TEXT).child(q.question.clone()),
                    )
                    .when(req.questions.len() > 1, |d| {
                        d.child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(format!(
                            "{} of {}",
                            page + 1,
                            req.questions.len()
                        )))
                    }),
            )
            .child(opts)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().flex_1().text_size(px(tk::T11)).text_color(tk::GHOST).child("Or type an answer below"))
                    .when(q.multi, |d| {
                        d.child(primary("q-next", "Next").on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| this.answer_next(id, rid.clone(), cx)),
                        ))
                    }),
            )
            .into_any_element()
    }

    // ----- new agent -----------------------------------------------------------------

    pub(crate) fn page_new(&self, cx: &mut Context<Self>) -> AnyElement {
        let form = &self.new_agent;
        let kind = form.kind.map(|k| k.label().to_string()).unwrap_or_else(|| "No agent".into());
        let project = form
            .project
            .and_then(|p| self.snap.projects.iter().find(|(id, _)| *id == p))
            .map(|(_, n)| n.clone())
            .unwrap_or_else(
                || if self.snap.projects.is_empty() { "No project".into() } else { "Pick a project".into() },
            );
        let summary = format!("{kind} · {project} · {}", if form.worktree { "own worktree" } else { "this folder" });
        let prompt = self.inputs.as_ref().map(|i| i.prompt.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .h(px(40.0))
                    .px(px(14.0))
                    .rounded(px(16.0))
                    .bg(tk::WASH)
                    .flex()
                    .items_center()
                    .text_size(px(tk::T13))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| this.take_focus(cx)),
                    )
                    .children(prompt.map(|p| div().flex_1().child(Input::new(&p)))),
            )
            .child(
                div()
                    .id("new-summary")
                    .h(px(22.0))
                    .px(px(4.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .cursor_pointer()
                    .text_size(px(tk::T12))
                    .text_color(tk::DIM)
                    .hover(|s| s.text_color(tk::TEXT))
                    .tooltip(tip("Change the agent, project or worktree"))
                    .when_some(form.kind, |d, k| d.child(mark(k, 13.0)))
                    .child(summary)
                    .child(icon(icons::ALT_ARROW_DOWN).size(px(13.0)).text_color(tk::FAINT))
                    .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                        let items = this.new_agent_menu();
                        this.open_menu(ev.position(), items, cx);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(tk::T11))
                            .text_color(tk::RED)
                            .child(form.error.clone().unwrap_or_default()),
                    )
                    .child(
                        primary("new-start", "Start")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.start_agent(window, cx))),
                    ),
            )
            .into_any_element()
    }

    // ----- music -----------------------------------------------------------------------

    pub(crate) fn play_button(&self, id: &'static str, size: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let playing = self.media.as_ref().is_some_and(|m| m.playing);
        div()
            .id(id)
            .size(px(size))
            .flex_none()
            .rounded_full()
            .bg(tk::WHITE)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(tk::WHITE_HOVER))
            .active(|s| s.opacity(0.85))
            .tooltip(tip(if playing { "Pause" } else { "Play" }))
            .child(
                icon(if playing { icons::MEDIA_PAUSE } else { icons::MEDIA_PLAY })
                    .size(px((size * 0.44).round()))
                    .text_color(tk::INK),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.toggle_play(cx);
            }))
    }

    pub(crate) fn toggle_play(&mut self, cx: &mut Context<Self>) {
        self.media_src.send(super::super::media::Control::PlayPause);
        if let Some(m) = &mut self.media {
            m.position = m.now();
            m.at = Instant::now();
            m.playing = !m.playing;
        }
        cx.notify();
    }

    pub(crate) fn page_music(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(m) = self.media.clone() else {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(icon(icons::MUSIC_NOTE).size(px(24.0)).text_color(tk::FAINT))
                .child(div().text_size(px(tk::T13)).text_color(tk::DIM).child("Nothing playing"))
                .child(
                    div()
                        .text_size(px(tk::T11))
                        .text_color(tk::FAINT)
                        .child("Play something in Spotify, a browser or any player."),
                )
                .child(
                    pill("open-spotify", "Open Spotify", false).on_click(
                        cx.listener(|_, _: &ClickEvent, _, _| super::super::media::open_player("Spotify.exe")),
                    ),
                )
                .into_any_element();
        };
        let now = m.now();
        let progress = self.scrub.filter(|s| *s >= 0.0).unwrap_or(m.progress());
        let art = if m.playing { 88.0 } else { 84.0 };
        let shown_volume = self.volume_shown_until.is_some();
        let this = cx.entity().downgrade();
        let hide_art = self.hero_flying(HeroKind::Art);
        div()
            .id("music")
            .group("music")
            .relative()
            .size_full()
            .rounded(px(tk::R_CARD))
            .overflow_hidden()
            .when_some(m.backdrop.clone(), |d, b| {
                d.child(img(b).absolute().top_0().left_0().size_full().object_fit(ObjectFit::Cover).opacity(0.9))
            })
            .child(
                div()
                    .relative()
                    .size_full()
                    .p(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(14.0))
                            .child(
                                div()
                                    .size(px(88.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(hide_art, |d| d.opacity(0.0))
                                    .child(self.art(art, 18.0)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.0))
                                    .child(
                                        div()
                                            .text_size(px(tk::T15))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(tk::TEXT)
                                            .truncate()
                                            .child(m.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(tk::T12))
                                            .text_color(tk::DIM)
                                            .truncate()
                                            .child(m.artist.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(tk::T11))
                                            .text_color(tk::FAINT)
                                            .truncate()
                                            .child(m.app.clone()),
                                    ),
                            )
                            .child(div().flex().flex_col().items_end().gap(px(8.0)).child(self.eq(18.0)).when(
                                shown_volume,
                                |d| {
                                    d.child(
                                        div()
                                            .w(px(6.0))
                                            .h(px(48.0))
                                            .rounded_full()
                                            .bg(tk::WASH_HOVER)
                                            .flex()
                                            .flex_col()
                                            .justify_end()
                                            .child(
                                                div()
                                                    .w_full()
                                                    .rounded_full()
                                                    .bg(tk::TEXT)
                                                    .h(gpui::relative(self.audio_state.volume.clamp(0.0, 1.0))),
                                            ),
                                    )
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(
                                div()
                                    .id("scrubber")
                                    .group("scrub")
                                    .relative()
                                    .h(px(14.0))
                                    .flex()
                                    .items_center()
                                    .when(m.can_seek, |d| d.cursor_pointer())
                                    .on_mouse_down(MouseButton::Left, {
                                        let this = this.clone();
                                        move |ev: &MouseDownEvent, _, cx| {
                                            let x = f32::from(ev.position.x);
                                            let _ = this.update(cx, |n, cx| {
                                                if let Some((left, w)) = n.scrub_bounds {
                                                    n.scrub = Some(((x - left) / w).clamp(0.0, 1.0));
                                                }
                                                cx.notify();
                                            });
                                        }
                                    })
                                    .child(
                                        canvas(|bounds, _, _| bounds, {
                                            let this = this.clone();
                                            move |bounds, _, _, cx| {
                                                let _ = this.update(cx, |n, _| {
                                                    n.scrub_bounds = Some((
                                                        f32::from(bounds.origin.x),
                                                        f32::from(bounds.size.width).max(1.0),
                                                    ));
                                                });
                                            }
                                        })
                                        .absolute()
                                        .size_full(),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .h(px(4.0))
                                            .group_hover("scrub", |s| s.h(px(7.0)))
                                            .rounded_full()
                                            .bg(tk::WASH_HOVER)
                                            .child(
                                                div()
                                                    .h_full()
                                                    .rounded_full()
                                                    .bg(tk::TEXT)
                                                    .w(gpui::relative(progress.clamp(0.0, 1.0))),
                                            ),
                                    )
                                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                                        if this.scrub.is_some()
                                            && ev.pressed_button == Some(MouseButton::Left)
                                            && let Some((left, w)) = this.scrub_bounds
                                        {
                                            this.scrub = Some(((f32::from(ev.position.x) - left) / w).clamp(0.0, 1.0));
                                            cx.notify();
                                        }
                                    }))
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            if let Some(f) = this.scrub.take().filter(|f| *f >= 0.0)
                                                && this.media.as_ref().is_some_and(|m| m.can_seek)
                                            {
                                                this.media_src.send(super::super::media::Control::Seek(f));
                                                if let Some(m) = &mut this.media {
                                                    m.position = m.duration.mul_f32(f);
                                                    m.at = Instant::now();
                                                }
                                            }
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .text_size(px(tk::T11))
                                    .text_color(tk::FAINT)
                                    .child(mins_secs(if self.scrub.is_some() {
                                        m.duration.mul_f32(progress)
                                    } else {
                                        now
                                    }))
                                    .child(if m.duration.is_zero() {
                                        String::new()
                                    } else {
                                        format!("-{}", mins_secs(m.duration.saturating_sub(now)))
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(24.0))
                            .child(ibtn("music-prev", icons::MEDIA_PREVIOUS, 38.0, tk::TEXT, "Previous").on_click(
                                cx.listener(|this, _: &ClickEvent, _, _| {
                                    this.media_src.send(super::super::media::Control::Previous)
                                }),
                            ))
                            .child(self.play_button("music-play", 48.0, cx))
                            .child(ibtn("music-next", icons::MEDIA_NEXT, 38.0, tk::TEXT, "Next").on_click(
                                cx.listener(|this, _: &ClickEvent, _, _| {
                                    this.media_src.send(super::super::media::Control::Next)
                                }),
                            )),
                    ),
            )
            // Shuffle and the output device, on hover.
            .child(
                div()
                    .absolute()
                    .top(px(10.0))
                    .right(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .invisible()
                    .group_hover("music", |s| s.visible())
                    .when(!self.audio_state.device.is_empty(), |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .px(px(8.0))
                                .h(px(24.0))
                                .rounded_full()
                                .bg(gpui::hsla(0.0, 0.0, 0.0, 0.3))
                                .text_size(px(tk::T11))
                                .text_color(tk::DIM)
                                .child(icon(icons::VOLUME_LOUD).size(px(12.0)).text_color(tk::FAINT))
                                .child(div().max_w(px(110.0)).truncate().child(self.audio_state.device.clone())),
                        )
                    })
                    .when_some(m.shuffle, |d, on| {
                        d.child(
                            ibtn(
                                "music-shuffle",
                                icons::MEDIA_SHUFFLE,
                                26.0,
                                if on { tk::GREEN } else { tk::DIM },
                                "Shuffle",
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.media_src.send(super::super::media::Control::Shuffle(!on));
                                cx.notify();
                            })),
                        )
                    }),
            )
            .into_any_element()
    }

    // ----- tray -----------------------------------------------------------------------

    /// The tray's items under the current filter: files first (newest
    /// first), then clipboard entries.
    pub(crate) fn tray_items(&self) -> Vec<TrayItem> {
        let mut out = Vec::new();
        let f = self.tray_filter;
        if matches!(f, TrayFilter::All | TrayFilter::Files) {
            out.extend((0..self.shelf.len()).map(TrayItem::File));
        }
        for (i, c) in self.clips.iter().enumerate() {
            let keep = match f {
                TrayFilter::All => true,
                TrayFilter::Files => false,
                TrayFilter::Text => matches!(c.kind(), ClipKind::Text | ClipKind::Code(_) | ClipKind::Color(..)),
                TrayFilter::Images => c.kind() == ClipKind::Image,
                TrayFilter::Links => matches!(c.kind(), ClipKind::Link(_)),
                TrayFilter::Pinned => c.pinned,
            };
            if keep {
                out.push(TrayItem::Clip(i));
            }
        }
        out
    }

    fn tray_tile(&mut self, item: TrayItem, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.tray_selected == Some(ix);
        let group = SharedString::from(format!("tile-{ix}"));
        let base = div()
            .id(("tile", ix))
            .group(group.clone())
            .relative()
            .h(px(84.0))
            .p(px(10.0))
            .rounded(px(16.0))
            .bg(tk::CARD)
            .when(selected, |d| d.border_1().border_color(tk::TEXT.opacity(0.5)))
            .overflow_hidden()
            .hover(|s| s.bg(tk::WASH_HOVER))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    let items = this.tile_menu(item);
                    this.open_menu(ev.position, items, cx);
                }),
            );
        let more = div().absolute().top(px(6.0)).right(px(6.0)).invisible().group_hover(group, |s| s.visible()).child(
            div()
                .id(("tile-more", ix))
                .size(px(24.0))
                .rounded_full()
                .bg(gpui::hsla(0.0, 0.0, 0.0, 0.45))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(gpui::hsla(0.0, 0.0, 0.0, 0.65)))
                .child(icon(icons::MORE_HORIZONTAL).size(px(14.0)).text_color(tk::TEXT))
                .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    let items = this.tile_menu(item);
                    this.open_menu(ev.position(), items, cx);
                })),
        );
        match item {
            TrayItem::File(i) => {
                let Some(path) = self.shelf.get(i).cloned() else { return div().into_any_element() };
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let size = std::fs::metadata(&path).ok().filter(|m| m.is_file()).map(|m| human_size(m.len()));
                let icon_img = self.icon_for(&path);
                let p_open = path.clone();
                base.cursor_grab()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, _| this.drag_from = Some((i, ev.position))),
                    )
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.drag_from = None))
                    .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                        let Some((ix, from)) = this.drag_from else { return };
                        if ev.pressed_button != Some(MouseButton::Left) {
                            this.drag_from = None;
                            return;
                        }
                        let d = ev.position - from;
                        if f32::from(d.x).abs() + f32::from(d.y).abs() < 6.0 {
                            return;
                        }
                        this.drag_from = None;
                        if let (Some(handle), Some(path)) =
                            (this.win.as_ref().and_then(|w| w.native), this.shelf.get(ix).cloned())
                        {
                            cx.spawn(async move |_, _| {
                                handle.drag_file(&path);
                            })
                            .detach();
                        }
                    }))
                    .on_click(cx.listener(move |_, ev: &ClickEvent, _, cx| {
                        if ev.click_count() >= 2 {
                            cx.open_with_system(&p_open);
                        }
                    }))
                    .tooltip(tip("Drag out · double-click opens"))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(match icon_img {
                                Some(im) => img(im).size(px(28.0)).into_any_element(),
                                None => icon(if path.is_dir() { icons::FOLDER } else { icons::DOCUMENT })
                                    .size(px(24.0))
                                    .text_color(tk::DIM)
                                    .into_any_element(),
                            })
                            .child(div().text_size(px(tk::T12)).text_color(tk::TEXT).truncate().child(name))
                            .child(
                                div()
                                    .text_size(px(tk::T11))
                                    .text_color(tk::FAINT)
                                    .child(size.unwrap_or_else(|| "Folder".into())),
                            ),
                    )
                    .child(more)
                    .into_any_element()
            }
            TrayItem::Clip(i) => {
                let Some(clip) = self.clips.get(i).cloned() else { return div().into_any_element() };
                let error = clip.as_text().is_some_and(super::super::data::looks_like_error);
                let body: AnyElement =
                    match (&clip.body, clip.kind()) {
                        (ClipBody::Image(im), _) => img(im.clone())
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .into_any_element(),
                        (ClipBody::Text(t), ClipKind::Link(host)) => div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.0))
                                    .child(icon(icons::GLOBAL).size(px(13.0)).text_color(tk::BLUE))
                                    .child(
                                        div()
                                            .text_size(px(tk::T12))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(tk::TEXT)
                                            .truncate()
                                            .child(host),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(tk::T11))
                                    .text_color(tk::FAINT)
                                    .line_clamp(2)
                                    .child(t.trim().to_string()),
                            )
                            .into_any_element(),
                        (ClipBody::Text(t), ClipKind::Color(r, g, b)) => div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(div().size(px(34.0)).rounded(px(10.0)).border_1().border_color(tk::HAIRLINE).bg(
                                gpui::Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 },
                            ))
                            .child(
                                div()
                                    .font_family(tk::MONO)
                                    .text_size(px(tk::T12))
                                    .text_color(tk::TEXT)
                                    .child(t.trim().to_string()),
                            )
                            .into_any_element(),
                        (ClipBody::Text(t), kind) => {
                            let tag = if error {
                                Some(("error", tk::RED))
                            } else if let ClipKind::Code(lang) = kind {
                                Some((lang, tk::BLUE))
                            } else {
                                None
                            };
                            let code = matches!(kind, ClipKind::Code(_)) || error;
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .when_some(tag, |d, (label, color)| {
                                    d.child(div().text_size(px(tk::T11)).text_color(color).child(label))
                                })
                                .child(
                                    div()
                                        .when(code, |d| d.font_family(tk::MONO).text_size(px(tk::T11)))
                                        .when(!code, |d| d.text_size(px(tk::T12)))
                                        .text_color(tk::TEXT)
                                        .line_clamp(if code { 3 } else { 4 })
                                        .child(t.trim().to_string()),
                                )
                                .into_any_element()
                        }
                    };
                let copy = clip.clone();
                base.on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                    if ev.click_count() >= 2 {
                        let (t, im) = match &copy.body {
                            ClipBody::Text(t) => (Some(t.clone()), None),
                            ClipBody::Image(im) => (None, Some(im.clone())),
                        };
                        this.paste_back(t, im, cx);
                        return;
                    }
                    cx.write_to_clipboard(match &copy.body {
                        ClipBody::Text(t) => ClipboardItem::new_string(t.clone()),
                        ClipBody::Image(im) => ClipboardItem::new_image(im),
                    });
                    this.tray_selected = Some(ix);
                    this.toast("Copied · Enter pastes");
                    cx.notify();
                }))
                .tooltip(tip("Click copies · double-click pastes"))
                .child(body)
                .when(clip.pinned, |d| {
                    d.child(
                        div()
                            .absolute()
                            .bottom(px(6.0))
                            .right(px(8.0))
                            .child(icon(icons::PIN).size(px(11.0)).text_color(tk::AMBER)),
                    )
                })
                .child(more)
                .into_any_element()
            }
        }
    }

    pub(crate) fn page_tray(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let label = match self.tray_filter {
            TrayFilter::All => "Everything",
            TrayFilter::Files => "Files",
            TrayFilter::Text => "Text",
            TrayFilter::Images => "Images",
            TrayFilter::Links => "Links",
            TrayFilter::Pinned => "Pinned",
        };
        let items = self.tray_items();
        let header = div()
            .h(px(30.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .id("tray-filter")
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(4.0))
                    .cursor_pointer()
                    .text_size(px(tk::T13))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(tk::TEXT)
                    .hover(|s| s.opacity(0.8))
                    .tooltip(tip("Filter, capture, clear"))
                    .child(label)
                    .child(icon(icons::ALT_ARROW_DOWN).size(px(13.0)).text_color(tk::FAINT))
                    .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                        let items = this.tray_menu();
                        this.open_menu(ev.position(), items, cx);
                    })),
            )
            .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(items.len().to_string()));
        if items.is_empty() {
            let hint = if self.prefs.clipboard {
                "Copy something, or drop files on the island"
            } else {
                "Drop files on the island · clipboard history is off in Settings"
            };
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(header)
                .child(
                    div()
                        .flex_1()
                        .rounded(px(tk::R_CARD))
                        .border_1()
                        .border_dashed()
                        .border_color(tk::HAIRLINE)
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(8.0))
                        .child(icon(icons::DOCUMENT_ADD).size(px(20.0)).text_color(tk::FAINT))
                        .child(div().text_size(px(tk::T12)).text_color(tk::DIM).child(hint)),
                )
                .into_any_element();
        }
        // A two-column masonry: tiles alternate between columns.
        let (mut left, mut right) = (
            div().flex_1().min_w_0().flex().flex_col().gap(px(8.0)),
            div().flex_1().min_w_0().flex().flex_col().gap(px(8.0)),
        );
        let key = self.key();
        for (ix, item) in items.into_iter().enumerate() {
            let tile = rise(self.tray_tile(item, ix, cx), (key, ix), ix);
            if ix % 2 == 0 {
                left = left.child(tile);
            } else {
                right = right.child(tile);
            }
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(header)
            .child(
                div()
                    .id("tray")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(div().flex().gap(px(8.0)).child(left).child(right)),
            )
            .into_any_element()
    }

    // ----- notes -----------------------------------------------------------------------

    pub(crate) fn page_notes(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div()
            .id("notes")
            .w(px(146.0))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                div()
                    .id("note-new")
                    .h(px(30.0))
                    .px(px(10.0))
                    .rounded(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .cursor_pointer()
                    .text_size(px(tk::T12))
                    .text_color(if self.selected_note.is_none() { tk::TEXT } else { tk::DIM })
                    .when(self.selected_note.is_none(), |d| d.bg(tk::WASH_HOVER))
                    .hover(|s| s.bg(tk::WASH))
                    .child(icon(icons::PLUS).size(px(14.0)).text_color(tk::DIM))
                    .child("New note")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.select_note(None, window, cx))),
            );
        for (i, n) in self.notes.iter().enumerate() {
            let id = n.id;
            let on = self.selected_note == Some(id);
            let group = SharedString::from(format!("note-row-{i}"));
            let title = n.title();
            list = list.child(
                div()
                    .id(("note", i))
                    .group(group.clone())
                    .relative()
                    .px(px(10.0))
                    .py(px(7.0))
                    .rounded(px(12.0))
                    .cursor_pointer()
                    .when(on, |d| d.bg(tk::WASH_HOVER))
                    .hover(|s| s.bg(tk::WASH))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, window, cx| this.select_note(Some(id), window, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            let items = this.note_menu(id);
                            this.open_menu(ev.position, items, cx);
                        }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .when(n.pinned, |d| d.child(icon(icons::PIN).size(px(10.0)).text_color(tk::AMBER)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(tk::T12))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(tk::TEXT)
                                    .child(if title.is_empty() { "Untitled".to_string() } else { title }),
                            ),
                    )
                    .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(match n.checklist() {
                        Some((d, t)) => format!("{d}/{t} done"),
                        None => ago(n.id),
                    }))
                    .child(
                        div()
                            .absolute()
                            .top(px(6.0))
                            .right(px(4.0))
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .child(ibtn(("note-more", i), icons::MORE_HORIZONTAL, 22.0, tk::DIM, "More").on_click(
                                cx.listener(move |this, ev: &ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    let items = this.note_menu(id);
                                    this.open_menu(ev.position(), items, cx);
                                }),
                            )),
                    ),
            );
        }
        // The editor: always editable; it saves itself as you type.
        let note_input = self.inputs.as_ref().map(|i| i.note.clone());
        let saved = self.note_dirty.is_none();
        let note = self.selected_note.and_then(|id| self.notes.iter().find(|n| n.id == id));
        let checklist = note.and_then(|n| n.checklist());
        // The checklist's items, tickable without editing the text.
        let checks: Vec<(usize, bool, String)> = note
            .map(|n| {
                n.text
                    .lines()
                    .enumerate()
                    .filter_map(|(i, l)| {
                        let t = l.trim_start();
                        let done = t.starts_with("- [x]");
                        (done || t.starts_with("- [ ]")).then(|| (i, done, t[5..].trim().to_string()))
                    })
                    .take(5)
                    .collect()
            })
            .unwrap_or_default();
        let pane = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(12.0))
                    .py(px(10.0))
                    .rounded(px(16.0))
                    .bg(tk::CARD)
                    .overflow_hidden()
                    .text_size(px(tk::T12))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| this.take_focus(cx)),
                    )
                    .children(note_input.map(|n| Textarea::new(&n))),
            )
            .when(!checks.is_empty(), |d| {
                d.child(div().flex().flex_col().gap(px(2.0)).children(checks.into_iter().map(|(line, done, text)| {
                    div()
                        .id(("check", line))
                        .h(px(24.0))
                        .px(px(6.0))
                        .rounded(px(8.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(tk::WASH))
                        .child(
                            div()
                                .size(px(14.0))
                                .rounded(px(5.0))
                                .border_1()
                                .border_color(if done { tk::GREEN } else { tk::DIM })
                                .when(done, |d| d.bg(tk::GREEN))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(done, |d| d.child(icon(icons::CHECK).size(px(10.0)).text_color(tk::INK))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(tk::T12))
                                .text_color(if done { tk::FAINT } else { tk::TEXT })
                                .child(text),
                        )
                        .on_click(
                            cx.listener(move |this, _: &ClickEvent, window, cx| this.toggle_check(line, window, cx)),
                        )
                })))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(tk::T11))
                    .text_color(tk::GHOST)
                    .child(div().flex_1().child(if saved { "Saved" } else { "Saving…" }))
                    .when_some(checklist, |d, (done, total)| d.child(format!("{done}/{total} checked"))),
            );
        div().size_full().flex().gap(px(10.0)).child(list).child(pane).into_any_element()
    }

    // ----- timer -----------------------------------------------------------------------

    pub(crate) fn page_timer(&self, cx: &mut Context<Self>) -> AnyElement {
        let running = self.timer.running();
        let remaining = self.timer.remaining();
        let last = running && remaining.as_secs() < 10;
        let pulse = if last { 0.6 + 0.4 * breathe(self.t() * 2.0) } else { 1.0 };
        let mut chips = div().flex().justify_center().gap(px(6.0));
        for (i, m) in [5u64, 15, 25, 45].iter().enumerate() {
            let m = *m;
            let on = self.timer.total == Duration::from_secs(m * 60);
            chips = chips.child(pill(("preset", i), format!("{m}m"), on).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.timer.set(Duration::from_secs(m * 60));
                    this.retarget(cx);
                    cx.notify();
                },
            )));
        }
        let linked =
            self.timer_link.and_then(|id| self.snap.agents.iter().find(|a| a.id == id)).map(|a| a.title.clone());
        let hide_ring = self.hero_flying(HeroKind::Ring);
        let has_menu = !self.timer_menu().is_empty();
        div()
            .relative()
            .size_full()
            .pt(px(8.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.0))
            .child(
                div()
                    .id("timer-face")
                    .when(hide_ring, |d| d.opacity(0.0))
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                        if this.timer.running() {
                            return;
                        }
                        let dy = match ev.delta {
                            ScrollDelta::Pixels(p) => f32::from(p.y),
                            ScrollDelta::Lines(l) => l.y * 20.0,
                        };
                        if dy.abs() > 1.0 {
                            this.timer.nudge(if dy > 0.0 { 1 } else { -1 });
                            cx.notify();
                        }
                    }))
                    .tooltip(tip("Scroll to set"))
                    .child(progress_ring(
                        if self.timer.active() { 1.0 - self.timer.progress() } else { 1.0 },
                        120.0,
                        4.0,
                        tk::AMBER.opacity(pulse),
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .opacity(pulse)
                            .child(roll(
                                &mins_secs(remaining),
                                "timer",
                                30.0,
                                if running { tk::TEXT } else { tk::DIM },
                                FontWeight::SEMIBOLD,
                            ))
                            .when_some(linked, |d, t| {
                                d.child(
                                    div()
                                        .max_w(px(90.0))
                                        .truncate()
                                        .text_size(px(tk::T11))
                                        .text_color(tk::FAINT)
                                        .child(format!("or when “{t}” ends")),
                                )
                            })
                            .into_any_element(),
                    )),
            )
            .child(chips)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .when(self.timer.active(), |d| {
                        d.child(link("timer-reset", "Reset").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.timer.reset();
                            this.timer_link = None;
                            this.retarget(cx);
                            cx.notify();
                        })))
                    })
                    .child(
                        primary(
                            "timer-go",
                            if running {
                                "Pause"
                            } else if self.timer.active() {
                                "Resume"
                            } else {
                                "Start"
                            },
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.timer.toggle();
                            this.retarget(cx);
                            cx.notify();
                        })),
                    ),
            )
            .when(has_menu, |d| {
                d.child(div().absolute().top(px(4.0)).right(px(0.0)).child(
                    ibtn("timer-more", icons::MORE_HORIZONTAL, 28.0, tk::DIM, "More").on_click(cx.listener(
                        |this, ev: &ClickEvent, _, cx| {
                            let items = this.timer_menu();
                            this.open_menu(ev.position(), items, cx);
                        },
                    )),
                ))
            })
            .into_any_element()
    }

    // ----- notifications -------------------------------------------------------------

    pub(crate) fn page_alerts(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.notices.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(icon(icons::BELL).size(px(22.0)).text_color(tk::FAINT))
                .child(div().text_size(px(tk::T13)).text_color(tk::DIM).child("Nothing new"))
                .child(
                    div()
                        .text_size(px(tk::T11))
                        .text_color(tk::FAINT)
                        .child("Finishes, failures and the timer show up here."),
                )
                .into_any_element();
        }
        let key = self.key();
        let mut col = div().id("alerts").size_full().overflow_y_scroll().flex().flex_col().gap(px(2.0)).child(
            div().flex().items_center().px(px(8.0)).pb(px(4.0)).child(div().flex_1()).child(
                ibtn("alerts-more", icons::MORE_HORIZONTAL, 26.0, tk::DIM, "More").on_click(cx.listener(
                    |this, ev: &ClickEvent, _, cx| {
                        this.open_menu(ev.position(), Notch::clear_notices_menu(), cx);
                    },
                )),
            ),
        );
        for (i, n) in self.notices.iter().enumerate() {
            let (glyph, color): (AnyElement, Hsla) = match (n.kind, n.agent) {
                (k, Some(kind)) => (avatar(kind, 26.0, None), if k == FlashKind::Failed { tk::RED } else { tk::GREEN }),
                (FlashKind::Timer, None) => {
                    (icon(icons::TIMER).size(px(15.0)).text_color(tk::AMBER).into_any_element(), tk::AMBER)
                }
                (FlashKind::Failed, None) => {
                    (icon(icons::DANGER_TRIANGLE).size(px(15.0)).text_color(tk::RED).into_any_element(), tk::RED)
                }
                _ => (icon(icons::CHECK).size(px(15.0)).text_color(tk::GREEN).into_any_element(), tk::GREEN),
            };
            let task = n.task;
            col = col.child(rise(
                div()
                    .id(("notice", i))
                    .h(px(42.0))
                    .px(px(8.0))
                    .rounded(px(tk::R_ROW))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .when(task.is_some(), |d| d.cursor_pointer().hover(|s| s.bg(tk::WASH)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if let Some(id) = task {
                            this.open_page(Page::Agent(id), Some(window), cx);
                        }
                    }))
                    .child(glyph)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(tk::T12))
                            .text_color(tk::TEXT)
                            .child(n.text.clone()),
                    )
                    .child(self.dot(color, 6.0, false))
                    .child(div().text_size(px(tk::T11)).text_color(tk::GHOST).child(ago(n.at_ms))),
                (key, i),
                i,
            ));
        }
        col.into_any_element()
    }

    // ----- dev servers -------------------------------------------------------------

    pub(crate) fn page_dev(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.ports.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(icon(icons::SERVER).size(px(22.0)).text_color(tk::FAINT))
                .child(div().text_size(px(tk::T13)).text_color(tk::DIM).child("No dev servers running"))
                .child(
                    div()
                        .text_size(px(tk::T11))
                        .text_color(tk::FAINT)
                        .child("Servers your agents start (Vite, Next, Django…) show up here."),
                )
                .into_any_element();
        }
        let mut col = div().id("dev").size_full().overflow_y_scroll().flex().flex_col().gap(px(2.0)).pt(px(4.0));
        for (i, p) in self.ports.iter().enumerate() {
            let url = format!("http://localhost:{}", p.port);
            let (port, pid) = (p.port, p.pid);
            let group = SharedString::from(format!("port-{i}"));
            col = col.child(
                div()
                    .id(("port", i))
                    .group(group.clone())
                    .h(px(48.0))
                    .px(px(10.0))
                    .rounded(px(tk::R_ROW))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(tk::WASH))
                    .tooltip(tip("Open in the browser"))
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            this.open_menu(ev.position, Notch::port_menu(port, pid), cx)
                        }),
                    )
                    .child(self.dot(tk::GREEN, 7.0, false))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_family(tk::MONO)
                                    .text_size(px(tk::T13))
                                    .text_color(tk::TEXT)
                                    .child(format!("localhost:{port}")),
                            )
                            .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(p.process.clone())),
                    )
                    .child(div().invisible().group_hover(group, |s| s.visible()).child(
                        ibtn(("port-more", i), icons::MORE_HORIZONTAL, 26.0, tk::DIM, "More").on_click(cx.listener(
                            move |this, ev: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.open_menu(ev.position(), Notch::port_menu(port, pid), cx);
                            },
                        )),
                    )),
            );
        }
        col.into_any_element()
    }
}
