//! The team's orchestration rules, independent of any UI.
//!
//! Agents in Dove work together through Dove's tool server: one thread hands
//! another a task (`delegate_task`), asks a teammate something
//! (`team_message`), or reads, steers and waits on any thread. This module
//! holds the parts that decide *what* happens (which agent runs a child, what
//! permission it may have, how deep work may nest, what a child reports back
//! and how results are worded for the agent that asked) so they can be tested
//! without a window. The app performs the decisions through the same paths a
//! click uses.

use std::collections::HashMap;
use std::sync::Arc;

use dove_proto::{AgentEvent, HarnessKind, PermissionMode, TaskId, TaskStatus};
use dove_store::{LinkStatus, Persona};

/// How deep delegated work may nest: a thread, its child, a grandchild.
pub const MAX_DEPTH: usize = 3;
/// How many delegated children one thread may have working at once.
pub const MAX_ACTIVE_CHILDREN: usize = 6;
/// Children finishing within this window of each other wake their parent once.
pub const DELIVERY_BATCH_MS: u64 = 1500;
/// A delegated result longer than this is cut, with a pointer to `thread_read`.
pub const MAX_SUMMARY_CHARS: usize = 6000;

/// The agent named in a tool call: an id (`claude_code`), a label
/// (`Claude Code`) or a common short name (`claude`, `gemini`).
pub fn resolve_agent(name: &str) -> Option<HarnessKind> {
    let n = name.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    if n.is_empty() {
        return None;
    }
    if let Some(k) = HarnessKind::from_id(&n) {
        return Some(k);
    }
    let aliases: &[(&str, HarnessKind)] = &[
        ("claude", HarnessKind::ClaudeCode),
        ("claudecode", HarnessKind::ClaudeCode),
        ("anthropic", HarnessKind::ClaudeCode),
        ("openai", HarnessKind::Codex),
        ("codex_cli", HarnessKind::Codex),
        ("gemini_cli", HarnessKind::Gemini),
        ("google", HarnessKind::Gemini),
        ("opencode", HarnessKind::OpenCode),
        ("cursor_agent", HarnessKind::Cursor),
        ("grok_build", HarnessKind::Grok),
        ("xai", HarnessKind::Grok),
        ("copilot_cli", HarnessKind::Copilot),
        ("factory", HarnessKind::Droid),
        ("factory_droid", HarnessKind::Droid),
        ("qwen_code", HarnessKind::Qwen),
        ("kilo_code", HarnessKind::Kilo),
        ("kimi_code", HarnessKind::Kimi),
    ];
    if let Some((_, k)) = aliases.iter().find(|(a, _)| *a == n) {
        return Some(*k);
    }
    HarnessKind::ALL.iter().copied().find(|k| k.label().to_ascii_lowercase().replace(['-', ' '], "_") == n)
}

/// How much a permission mode lets an agent do without asking, lowest first.
fn reach(m: PermissionMode) -> u8 {
    match m {
        PermissionMode::Plan => 0,
        PermissionMode::Default => 1,
        PermissionMode::AcceptEdits => 2,
        PermissionMode::Bypass => 3,
    }
}

/// The mode a child runs in: what was asked for, never more than the
/// thread that started it may do. Work handed on can't gain permissions.
pub fn cap_mode(parent: PermissionMode, requested: Option<PermissionMode>) -> PermissionMode {
    match requested {
        Some(r) if reach(r) <= reach(parent) => r,
        _ => parent,
    }
}

pub fn parse_mode(s: &str) -> Option<PermissionMode> {
    Some(match s.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
        "plan" | "read_only" | "readonly" => PermissionMode::Plan,
        "default" | "ask" | "approval_required" | "supervised" => PermissionMode::Default,
        "accept_edits" | "auto_accept_edits" | "acceptedits" => PermissionMode::AcceptEdits,
        "bypass" | "full_access" | "full" => PermissionMode::Bypass,
        _ => return None,
    })
}

pub fn mode_label(m: PermissionMode) -> &'static str {
    match m {
        PermissionMode::Plan => "plan",
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "accept_edits",
        PermissionMode::Bypass => "bypass",
    }
}

/// Who runs a delegated child, given the request, the teammate it names,
/// the caller's own agent and what is installed.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub harness: HarnessKind,
    pub model: Option<String>,
}

pub fn pick_agent(
    agent: Option<&str>,
    model: Option<&str>,
    teammate: Option<&Persona>,
    caller: HarnessKind,
    installed: &[HarnessKind],
) -> Result<Pick, String> {
    let model = model.map(str::trim).filter(|m| !m.is_empty()).map(str::to_string);
    if let Some(name) = agent.map(str::trim).filter(|a| !a.is_empty()) {
        let Some(kind) = resolve_agent(name) else {
            return Err(format!("No agent called \"{name}\". Call team_capabilities for the installed agents."));
        };
        if !installed.contains(&kind) {
            return Err(format!("{} is not installed on this computer.", kind.label()));
        }
        // A teammate's preferred model only applies on its preferred agent.
        let model = model.or_else(|| teammate.filter(|t| t.harness == Some(kind)).and_then(|t| t.model.clone()));
        return Ok(Pick { harness: kind, model });
    }
    if let Some(kind) = teammate.and_then(|t| t.harness).filter(|k| installed.contains(k)) {
        return Ok(Pick { harness: kind, model: model.or_else(|| teammate.and_then(|t| t.model.clone())) });
    }
    if installed.contains(&caller) {
        return Ok(Pick { harness: caller, model });
    }
    installed
        .first()
        .map(|k| Pick { harness: *k, model: None })
        .ok_or_else(|| "No agent that Keplorayx can run is installed.".to_string())
}

/// How many links up from `task` to a thread nobody started: 0 for a
/// thread the user started, 1 for its child, and so on.
pub fn depth(task: TaskId, parent_of: &HashMap<TaskId, TaskId>) -> usize {
    let mut d = 0;
    let mut at = task;
    while let Some(p) = parent_of.get(&at) {
        d += 1;
        at = *p;
        if d > 64 {
            break; // a cycle in a hand-edited database; don't spin
        }
    }
    d
}

/// The link status a thread's status means.
pub fn link_status(s: TaskStatus) -> LinkStatus {
    match s {
        TaskStatus::Done | TaskStatus::Idle => LinkStatus::Completed,
        TaskStatus::Failed => LinkStatus::Failed,
        TaskStatus::NeedsInput => LinkStatus::NeedsInput,
        TaskStatus::Archived => LinkStatus::Cancelled,
        TaskStatus::Running | TaskStatus::Draft => LinkStatus::Running,
    }
}

/// Text blocks of the last turn: what the agent said after the last user
/// message, one entry per streamed block.
fn last_turn_blocks(events: &[AgentEvent]) -> (Vec<String>, Option<String>) {
    let start = events.iter().rposition(|e| matches!(e, AgentEvent::UserMessage { .. })).map_or(0, |i| i + 1);
    let mut blocks = Vec::new();
    let mut cur = String::new();
    let mut error = None;
    for e in &events[start..] {
        match e {
            AgentEvent::TextDelta { text } => cur.push_str(text),
            AgentEvent::BlockEnd | AgentEvent::ToolCall { .. } => {
                if !cur.trim().is_empty() {
                    blocks.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
            AgentEvent::Error { message } => error = Some(message.to_string()),
            _ => {}
        }
    }
    if !cur.trim().is_empty() {
        blocks.push(cur);
    }
    (blocks, error)
}

/// What a thread reported at the end of its last turn: its final text, or
/// the error it stopped on. The final block is the answer; earlier blocks
/// are narration between tool calls.
pub fn final_answer(events: &[AgentEvent]) -> Option<String> {
    let (blocks, error) = last_turn_blocks(events);
    match (blocks.last(), error) {
        (Some(b), _) => Some(clip(b.trim(), MAX_SUMMARY_CHARS)),
        (None, Some(e)) => Some(format!("Error: {e}")),
        (None, None) => None,
    }
}

/// `s`, cut to `max` characters on a char boundary with a marker.
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}\n… (cut; read the rest with thread_read)")
}

/// A readable account of a thread for another agent: each turn's request,
/// the tools used, and what the agent answered. `after` skips earlier turns
/// (turn numbers start at 1); the result is at most `max_chars` long, newest
/// turns kept when it must be cut.
pub fn timeline(events: &[AgentEvent], after: usize, max_chars: usize) -> (String, usize) {
    struct Turn {
        ask: String,
        tools: Vec<String>,
        text: String,
        error: Option<String>,
    }
    let mut turns: Vec<Turn> = Vec::new();
    for e in events {
        match e {
            AgentEvent::UserMessage { text } => turns.push(Turn {
                ask: crate::strip_persona(text).trim().to_string(),
                tools: Vec::new(),
                text: String::new(),
                error: None,
            }),
            AgentEvent::TextDelta { text } => {
                if turns.is_empty() {
                    turns.push(Turn { ask: String::new(), tools: Vec::new(), text: String::new(), error: None });
                }
                turns.last_mut().unwrap().text.push_str(text);
            }
            AgentEvent::BlockEnd => {
                if let Some(t) = turns.last_mut()
                    && !t.text.ends_with("\n\n")
                    && !t.text.is_empty()
                {
                    t.text.push_str("\n\n");
                }
            }
            AgentEvent::ToolCall { title, .. } => {
                if let Some(t) = turns.last_mut()
                    && t.tools.len() < 40
                {
                    t.tools.push(title.to_string());
                }
            }
            AgentEvent::ToolTitle { .. } => {}
            AgentEvent::Error { message } => {
                if let Some(t) = turns.last_mut() {
                    t.error = Some(message.to_string());
                }
            }
            _ => {}
        }
    }
    let total = turns.len();
    let mut parts: Vec<String> = Vec::new();
    for (i, t) in turns.iter().enumerate().skip(after) {
        let mut s = format!("### Turn {}\n", i + 1);
        if !t.ask.is_empty() {
            s.push_str(&format!("Request:\n{}\n", t.ask));
        }
        if !t.tools.is_empty() {
            s.push_str(&format!("Tools: {}\n", t.tools.join("; ")));
        }
        if !t.text.trim().is_empty() {
            s.push_str(&format!("Answer:\n{}\n", t.text.trim()));
        }
        if let Some(e) = &t.error {
            s.push_str(&format!("Error: {e}\n"));
        }
        parts.push(s);
    }
    // Keep the newest turns when everything doesn't fit.
    let mut out: Vec<String> = Vec::new();
    let mut used = 0;
    for p in parts.into_iter().rev() {
        let len = p.chars().count();
        if used + len > max_chars && !out.is_empty() {
            out.push(format!("… earlier turns cut; pass after to read from a turn (this thread has {total})\n"));
            break;
        }
        used += len;
        out.push(if len > max_chars { clip(&p, max_chars) } else { p });
    }
    out.reverse();
    (out.join("\n"), total)
}

/// Who a message or task comes from, as the receiving agent reads it. Angle
/// brackets are dropped so the label can sit in a tag attribute.
pub fn sender_label(teammate: Option<&Persona>, thread_title: &str, agent: HarnessKind) -> String {
    let thread_title = thread_title.replace(['<', '>'], "");
    let thread_title = thread_title.as_str();
    match teammate {
        Some(p) if !p.role.trim().is_empty() => format!("{} ({}), in \"{thread_title}\"", p.name.trim(), p.role.trim()),
        Some(p) => format!("{}, in \"{thread_title}\"", p.name.trim()),
        None => format!("the {} thread \"{thread_title}\"", agent.label()),
    }
}

/// A delegated task, as the child agent receives it. The child gets only
/// this (not the parent's conversation), so it says where it came from and
/// that its final answer is the report.
pub fn delegation_prompt(from: &str, task: &str, isolated: bool) -> String {
    let place = if isolated {
        "You work in your own git worktree, branched from the parent's latest commit (uncommitted edits were not copied)."
    } else {
        "You share the parent's working folder, so other agents may be editing files too: keep changes focused."
    };
    format!(
        "<dove-task from=\"{from}\">\n{}\n</dove-task>\n\n{place} This task was handed to you in Keplorayx by {from}. \
         When you are done, end your turn with a clear, self-contained report: what you did, what you found, and \
         anything left open. That final message is delivered back automatically.",
        task.trim()
    )
}

/// A teammate's message, as the receiving teammate reads it.
pub fn message_prompt(from: &str, message: &str, wants_reply: bool) -> String {
    let reply = if wants_reply {
        " Reply by finishing your turn: your final message goes back to them automatically."
    } else {
        ""
    };
    format!(
        "<dove-message from=\"{from}\">\n{}\n</dove-message>\n\nA message from {from} in Keplorayx.{reply}",
        message.trim()
    )
}

/// One finished (or stuck) piece of work, for a team update.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub thread: TaskId,
    pub title: String,
    /// "Kai (Builder) on Codex", "Claude Code".
    pub who: String,
    pub status: LinkStatus,
    pub summary: Option<String>,
    pub delegated: bool,
}

/// The message that wakes a parent with its children's results, all at once.
pub fn team_update(reports: &[Report]) -> String {
    let mut out = String::from("<dove-team-update>\n");
    for r in reports {
        let what = match (r.delegated, r.status) {
            (true, LinkStatus::Completed) => "finished the task you handed over",
            (true, LinkStatus::Failed) => "stopped with an error on the task you handed over",
            (true, LinkStatus::Cancelled) => "was cancelled",
            (true, LinkStatus::NeedsInput) => "is waiting for the user (an approval or a question) on your task",
            (true, LinkStatus::Running) => "is still working",
            (false, LinkStatus::NeedsInput) => "is waiting for the user before it can reply",
            (false, LinkStatus::Failed) => "couldn't reply (it stopped with an error)",
            (false, _) => "replied",
        };
        out.push_str(&format!("\n## {} {what}\nThread \"{}\" (id {})\n", r.who, r.title, r.thread.0));
        if let Some(s) = r.summary.as_deref().filter(|s| !s.trim().is_empty()) {
            out.push_str(&format!("\n{}\n", s.trim()));
        }
    }
    out.push_str(
        "\n</dove-team-update>\n\nThis is an automatic update from Keplorayx, not a message from the user. Continue \
         your work with these results. Use thread_read with a thread id for the full detail, or thread_send to \
         follow up.",
    );
    out
}

/// Whether `text` is an automatic team update (the chat shows it as a card).
pub fn is_team_update(text: &str) -> bool {
    text.trim_start().starts_with("<dove-team-update>")
}

/// A message one agent sent another, as the chat shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum TeamText {
    /// A delegated task, from `from`.
    Task { from: String, body: String },
    /// A teammate's message, from `from`.
    Message { from: String, body: String },
    /// Results delivered back to this thread.
    Update { body: String },
}

/// Recognise Dove's team framing in a user-role message.
pub fn parse_team_text(text: &str) -> Option<TeamText> {
    let t = text.trim_start();
    let tagged = |open: &str, close: &str| -> Option<(String, String)> {
        let rest = t.strip_prefix(open)?;
        let (attrs, after) = rest.split_once('>')?;
        if !(attrs.is_empty() || attrs.starts_with(char::is_whitespace)) {
            return None; // another tag that merely starts the same way
        }
        let from = attrs.trim().strip_prefix("from=\"").and_then(|s| s.strip_suffix('"')).unwrap_or("").to_string();
        let body = after.split(close).next().unwrap_or("").trim().to_string();
        Some((from, body))
    };
    if let Some(rest) = t.strip_prefix("<dove-team-update>") {
        let body = rest.split("</dove-team-update>").next().unwrap_or("").trim().to_string();
        return Some(TeamText::Update { body });
    }
    if let Some((from, body)) = tagged("<dove-task", "</dove-task>") {
        return Some(TeamText::Task { from, body });
    }
    if let Some((from, body)) = tagged("<dove-message", "</dove-message>") {
        return Some(TeamText::Message { from, body });
    }
    None
}

/// The teammate most recently busy in `project`, for `team_message`: their
/// newest thread there, skipping the caller's own and finished-archived ones.
pub fn reply_thread<'a>(
    candidates: impl Iterator<Item = (&'a dove_store::Task, &'a str)>,
    teammate: &str,
    project: dove_proto::ProjectId,
    caller: TaskId,
) -> Option<TaskId> {
    candidates
        .filter(|(t, p)| {
            *p == teammate && t.project_id == project && t.id != caller && t.status != TaskStatus::Archived
        })
        .max_by_key(|(t, _)| t.updated_at)
        .map(|(t, _)| t.id)
}

/// `name` matched against the team: id, then name, ignoring case.
pub fn find_teammate<'a>(team: &'a [Persona], name: &str) -> Option<&'a Persona> {
    let n = name.trim();
    team.iter().find(|p| p.id == n).or_else(|| team.iter().find(|p| p.name.trim().eq_ignore_ascii_case(n))).or_else(
        || {
            let lower = n.to_ascii_lowercase();
            let mut hits = team.iter().filter(|p| p.name.to_ascii_lowercase().starts_with(&lower));
            match (hits.next(), hits.next()) {
                (Some(p), None) => Some(p),
                _ => None,
            }
        },
    )
}

/// Shared text for `Arc<str>` events in tests.
#[doc(hidden)]
pub fn text(s: &str) -> Arc<str> {
    Arc::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dove_proto::ToolKind;

    fn persona(id: &str, name: &str, role: &str, harness: Option<HarnessKind>) -> Persona {
        Persona {
            id: id.into(),
            name: name.into(),
            role: role.into(),
            avatar: String::new(),
            hue: 0.5,
            harness,
            model: None,
            instructions: String::new(),
            created_at: 0,
        }
    }

    #[test]
    fn agents_resolve_by_id_label_and_short_name() {
        assert_eq!(resolve_agent("claude"), Some(HarnessKind::ClaudeCode));
        assert_eq!(resolve_agent("Claude Code"), Some(HarnessKind::ClaudeCode));
        assert_eq!(resolve_agent("claude_code"), Some(HarnessKind::ClaudeCode));
        assert_eq!(resolve_agent("codex"), Some(HarnessKind::Codex));
        assert_eq!(resolve_agent("Gemini CLI"), Some(HarnessKind::Gemini));
        assert_eq!(resolve_agent("gemini"), Some(HarnessKind::Gemini));
        assert_eq!(resolve_agent("opencode"), Some(HarnessKind::OpenCode));
        assert_eq!(resolve_agent("open-code"), Some(HarnessKind::OpenCode));
        assert_eq!(resolve_agent("Factory Droid"), Some(HarnessKind::Droid));
        assert_eq!(resolve_agent("pi"), Some(HarnessKind::Pi));
        assert_eq!(resolve_agent("nope"), None);
        assert_eq!(resolve_agent("  "), None);
    }

    #[test]
    fn children_never_gain_permissions() {
        use PermissionMode::*;
        assert_eq!(cap_mode(Default, None), Default);
        assert_eq!(cap_mode(Default, Some(Bypass)), Default, "no escalation");
        assert_eq!(cap_mode(Default, Some(AcceptEdits)), Default);
        assert_eq!(cap_mode(Default, Some(Plan)), Plan, "narrower is fine");
        assert_eq!(cap_mode(Bypass, Some(AcceptEdits)), AcceptEdits);
        assert_eq!(cap_mode(Plan, Some(Default)), Plan);
        assert_eq!(cap_mode(AcceptEdits, None), AcceptEdits);
        assert_eq!(parse_mode("full-access"), Some(Bypass));
        assert_eq!(parse_mode("read only"), Some(Plan));
        assert_eq!(parse_mode("approval-required"), Some(Default));
        assert_eq!(parse_mode("whatever"), None);
    }

    #[test]
    fn the_right_agent_runs_a_child() {
        let installed = [HarnessKind::ClaudeCode, HarnessKind::Codex];
        let kai =
            Persona { model: Some("gpt-5.5".into()), ..persona("kai", "Kai", "Builder", Some(HarnessKind::Codex)) };
        let any = persona("ava", "Ava", "Reviewer", None);
        // Explicit agent wins; the teammate's model only rides along on its own agent.
        let p = pick_agent(Some("codex"), None, Some(&kai), HarnessKind::ClaudeCode, &installed).unwrap();
        assert_eq!(p, Pick { harness: HarnessKind::Codex, model: Some("gpt-5.5".into()) });
        let p = pick_agent(Some("claude"), None, Some(&kai), HarnessKind::Codex, &installed).unwrap();
        assert_eq!(p, Pick { harness: HarnessKind::ClaudeCode, model: None });
        // A teammate's preference, then the caller's own agent.
        let p = pick_agent(None, None, Some(&kai), HarnessKind::ClaudeCode, &installed).unwrap();
        assert_eq!(p.harness, HarnessKind::Codex);
        let p = pick_agent(None, Some("haiku"), Some(&any), HarnessKind::ClaudeCode, &installed).unwrap();
        assert_eq!(p, Pick { harness: HarnessKind::ClaudeCode, model: Some("haiku".into()) });
        // Errors name the problem.
        assert!(
            pick_agent(Some("gemini"), None, None, HarnessKind::ClaudeCode, &installed)
                .unwrap_err()
                .contains("not installed")
        );
        assert!(
            pick_agent(Some("xyz"), None, None, HarnessKind::ClaudeCode, &installed).unwrap_err().contains("No agent")
        );
        assert!(pick_agent(None, None, None, HarnessKind::ClaudeCode, &[]).is_err());
    }

    #[test]
    fn depth_counts_links_and_survives_cycles() {
        let (a, b, c) = (TaskId::new(), TaskId::new(), TaskId::new());
        let mut parent_of = HashMap::new();
        assert_eq!(depth(a, &parent_of), 0);
        parent_of.insert(b, a);
        parent_of.insert(c, b);
        assert_eq!(depth(c, &parent_of), 2);
        parent_of.insert(a, c);
        assert!(depth(a, &parent_of) > MAX_DEPTH, "a cycle stops, deep");
    }

    fn turn(user: &str, blocks: &[&str]) -> Vec<AgentEvent> {
        let mut v = vec![AgentEvent::UserMessage { text: text(user) }];
        for (i, b) in blocks.iter().enumerate() {
            v.push(AgentEvent::TextDelta { text: text(b) });
            v.push(AgentEvent::BlockEnd);
            if i + 1 < blocks.len() {
                v.push(AgentEvent::ToolCall {
                    call_id: format!("c{i}"),
                    name: "Edit".into(),
                    kind: ToolKind::Edit,
                    title: text("Edit src/auth.rs"),
                    input: serde_json::json!({}),
                });
            }
        }
        v.push(AgentEvent::TurnEnded { usage: None, is_error: false });
        v
    }

    #[test]
    fn the_final_answer_is_the_last_block_of_the_last_turn() {
        let mut ev = turn("first", &["old answer"]);
        ev.extend(turn("second", &["Looking at the code…", "Fixed the refresh race in src/auth.rs; tests pass."]));
        assert_eq!(final_answer(&ev).unwrap(), "Fixed the refresh race in src/auth.rs; tests pass.");
        let mut failed = vec![AgentEvent::UserMessage { text: text("go") }];
        failed.push(AgentEvent::Error { message: text("rate limited") });
        assert_eq!(final_answer(&failed).unwrap(), "Error: rate limited");
        assert_eq!(final_answer(&[AgentEvent::UserMessage { text: text("go") }]), None);
        let long = "x".repeat(MAX_SUMMARY_CHARS + 10);
        assert!(final_answer(&turn("q", &[&long])).unwrap().contains("thread_read"));
    }

    #[test]
    fn timelines_show_requests_tools_answers_and_keep_the_newest() {
        let mut ev = turn("Fix the login bug", &["Checking.", "Done."]);
        ev.extend(turn("Now add a test", &["Added tests/login.rs."]));
        let (t, total) = timeline(&ev, 0, 10_000);
        assert_eq!(total, 2);
        assert!(t.contains("### Turn 1") && t.contains("Request:\nFix the login bug"));
        assert!(t.contains("Tools: Edit src/auth.rs"));
        assert!(t.contains("### Turn 2") && t.contains("Added tests/login.rs."));
        let (later, _) = timeline(&ev, 1, 10_000);
        assert!(!later.contains("### Turn 1") && later.contains("### Turn 2"));
        let (cut, _) = timeline(&ev, 0, 60);
        assert!(cut.contains("### Turn 2") && !cut.contains("### Turn 1"), "{cut}");
        assert!(cut.contains("earlier turns cut"));
    }

    #[test]
    fn messages_say_who_sent_them_and_how_to_reply() {
        let ava = persona("ava", "Ava", "Reviewer", None);
        let from = sender_label(Some(&ava), "Auth fix", HarnessKind::ClaudeCode);
        assert_eq!(from, "Ava (Reviewer), in \"Auth fix\"");
        assert_eq!(sender_label(None, "Auth fix", HarnessKind::Codex), "the Codex thread \"Auth fix\"");
        let d = delegation_prompt(&from, "  Review the diff  ", true);
        assert!(d.starts_with("<dove-task from=") && d.contains("Review the diff\n</dove-task>"));
        assert!(d.contains("own git worktree") && d.contains("delivered back automatically"));
        assert!(delegation_prompt(&from, "x", false).contains("share the parent's working folder"));
        let m = message_prompt(&from, "Can you check the migration?", true);
        assert!(m.contains("<dove-message") && m.contains("goes back to them automatically"));
        assert!(!message_prompt(&from, "FYI", false).contains("goes back"));
    }

    #[test]
    fn team_updates_batch_reports_and_read_as_automatic() {
        let r = |delegated, status, summary: Option<&str>| Report {
            thread: TaskId::new(),
            title: "Review".into(),
            who: "Kai (Builder) on Codex".into(),
            status,
            summary: summary.map(str::to_string),
            delegated,
        };
        let msg = team_update(&[
            r(true, LinkStatus::Completed, Some("All good.")),
            r(true, LinkStatus::NeedsInput, None),
            r(false, LinkStatus::Completed, Some("Yes, the migration is safe.")),
        ]);
        assert!(is_team_update(&msg));
        assert!(msg.contains("finished the task you handed over") && msg.contains("All good."));
        assert!(msg.contains("waiting for the user"));
        assert!(msg.contains("replied") && msg.contains("the migration is safe"));
        assert!(msg.contains("not a message from the user"));
        assert!(!is_team_update("hello"));
        match parse_team_text(&msg).unwrap() {
            TeamText::Update { body } => assert!(body.contains("All good.") && !body.contains("</dove-team-update>")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn team_framing_parses_back_for_the_chat() {
        let d = delegation_prompt("Ava (Reviewer), in \"Auth\"", "Check the tests", false);
        assert_eq!(
            parse_team_text(&d),
            Some(TeamText::Task { from: "Ava (Reviewer), in \"Auth\"".into(), body: "Check the tests".into() })
        );
        let m = message_prompt("Kai", "Ready for review?", true);
        assert_eq!(
            parse_team_text(&m),
            Some(TeamText::Message { from: "Kai".into(), body: "Ready for review?".into() })
        );
        assert_eq!(parse_team_text("Just a normal message"), None);
        assert_eq!(parse_team_text("<dove-taskish>"), None);
    }

    #[test]
    fn teammates_are_found_by_id_name_or_unique_prefix() {
        let team = vec![persona("1", "Ava", "", None), persona("2", "Avery", "", None), persona("3", "Kai", "", None)];
        assert_eq!(find_teammate(&team, "3").unwrap().name, "Kai");
        assert_eq!(find_teammate(&team, "ava").unwrap().id, "1", "an exact name beats a prefix");
        assert_eq!(find_teammate(&team, "ka").unwrap().id, "3");
        assert!(find_teammate(&team, "av").is_none(), "ambiguous prefix");
        assert!(find_teammate(&team, "zed").is_none());
    }

    #[test]
    fn replies_go_to_the_teammates_newest_thread_in_the_project() {
        use dove_store::Task;
        let project = dove_proto::ProjectId::new();
        let mk = |status, at, proj| Task {
            id: TaskId::new(),
            project_id: proj,
            title: "t".into(),
            harness: HarnessKind::Codex,
            model: None,
            status,
            worktree: None,
            branch: None,
            native_session_id: None,
            base: None,
            pinned: false,
            updated_at: at,
            prompt: None,
            unread: false,
            automation: None,
        };
        let old = mk(TaskStatus::Done, 1, project);
        let new = mk(TaskStatus::Done, 5, project);
        let archived = mk(TaskStatus::Archived, 9, project);
        let elsewhere = mk(TaskStatus::Done, 9, dove_proto::ProjectId::new());
        let caller = mk(TaskStatus::Running, 10, project);
        let rows = [(&old, "kai"), (&new, "kai"), (&archived, "kai"), (&elsewhere, "kai"), (&caller, "kai")];
        assert_eq!(reply_thread(rows.iter().copied(), "kai", project, caller.id), Some(new.id));
        assert_eq!(reply_thread(rows.iter().copied(), "ava", project, caller.id), None);
    }

    #[test]
    fn link_status_follows_the_thread() {
        assert_eq!(link_status(TaskStatus::Done), LinkStatus::Completed);
        assert_eq!(link_status(TaskStatus::Failed), LinkStatus::Failed);
        assert_eq!(link_status(TaskStatus::NeedsInput), LinkStatus::NeedsInput);
        assert_eq!(link_status(TaskStatus::Running), LinkStatus::Running);
        assert!(LinkStatus::Completed.terminal() && !LinkStatus::NeedsInput.terminal());
    }
}
