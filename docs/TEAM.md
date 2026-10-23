# Team and orchestration

Teammates are people you brief once (a name, a role, standing instructions,
notes they keep between threads) who work on any agent Dove runs. Every
thread in Dove, whether a teammate runs it or not, can hand work to any
agent and model, message teammates, and read, steer, wait on or stop other
threads. Architecture:
[ARCHITECTURE.md › Team and orchestration](../ARCHITECTURE.md#team-and-orchestration).
Code: `crates/dove-core/src/orchestra.rs` (the rules, no UI),
`apps/dove/src/team_tools.rs` (the tool definitions),
`apps/dove/src/orchestra.rs` (the app side) and `apps/dove/src/team.rs` (the
sidebar section and Settings › Team).

## Using it

1. Settings › Team (or the **Team** heading in the sidebar): **Add starter
   team** creates a Lead, Builder, Reviewer and Designer, or create your own.
   A teammate needs no agent or model. Leave **Prefers** on **Any agent** and
   each of their threads runs on whatever you pick in the composer, or on
   whatever another agent asks for when it hands them work. Pick an agent
   (and model) only if you want that as their default.
2. In the sidebar, click a teammate to show their threads underneath. These
   are ordinary thread rows: click to open, double-click to rename, the
   archive button, the right-click menu. With no threads yet, the click
   starts one as them. `+` starts a new thread as them; the gear opens them
   in Settings › Team.
3. Inside any thread, ask for collaboration in plain words: "have the
   Reviewer check this on Codex", "get a second opinion from Gemini 2.5 Pro",
   "ask the Designer what they think of the empty state", "split the
   migration across two agents in their own worktrees". The agent uses the
   team tools below.
4. Work handed over shows up as its own thread (in the thread list, and
   under the teammate if one runs it). Its first message is a **Task from …**
   card. Replies come back to the thread that asked, as a **Team update**
   card. A message between teammates is a **Message from …** card.

## The tools

Every agent Dove starts gets these through Dove's tool server, beside the
browser tools (they stay available with the agent browser turned off).

| Tool | What it does |
|---|---|
| `team_capabilities` | Your own thread (id, agent, model, teammate, permission, depth, children), the installed agents with their models and reasoning levels, the teammates (preferred agent, recent threads) and the limits. |
| `delegate_task` | Start a child thread on any agent and model, optionally as a teammate, in your folder or its own git worktree, with a permission mode no broader than yours. `mode: "async"` (default) returns at once and Dove wakes you with the result; `mode: "wait"` returns the result in the call. `request_key` makes retries idempotent. |
| `task_status` | How a delegated task stands, with its result once finished. |
| `task_cancel` | Stop a delegated task and everything it delegated in turn. |
| `team_message` | Message a teammate: their most recent thread in this project, or a new thread as them. Their reply comes back to you (or in the call with `mode: "wait"`). |
| `thread_list` | Threads in this project (or all), filterable by title, status and teammate. |
| `thread_read` | A thread's turns: each request, the tools used, the answer. Long threads keep the newest turns; `after` pages. |
| `thread_send` | Send to any thread: steers a working one (or queues on agents that can't be steered), starts a turn on an idle one. `wants_reply` routes its answer back. |
| `thread_wait` | Wait until a thread stops working or the timeout passes. Waiting never stops it. |
| `thread_interrupt` | Stop a thread's current turn, like pressing Stop. |

The read-only tools (`team_capabilities`, `task_status`, `thread_list`,
`thread_read`, `thread_wait`) run without an approval prompt. The others
follow the thread's permission mode like any tool.

## How results come back

- A child gets only the task text, framed so it knows who sent it and that
  its final message is the report. It never sees the parent's conversation.
- When a child (or a teammate asked for a reply) finishes, fails, or stops
  to wait for you, its final answer is queued for the parent. Results that
  land within 1.5 s of each other go out together as one team update.
- The update is sent to the parent like a message: it steers the parent if
  it's working (or waits its turn on agents that can't be steered), and
  starts a turn if it's idle. The parent's view is opened if needed.
- A result the parent already took (`mode: "wait"`, or `task_status` on a
  finished task) isn't delivered again. A child that stopped for your
  approval reports that once, and reports again when it finishes.
- Delivery survives closed views: a sweep every 0.7 s settles linked
  threads from the store, so a child whose view hibernated still reports.

## Guardrails

- **No escalation.** A child's permission mode is the parent's or narrower
  (Plan < Default < Accept edits < Bypass). `thread_send` refuses a target
  that runs in a broader mode than the sender.
- **Depth and fan-out.** Delegation nests at most 3 deep. A thread may have
  6 delegated tasks working at once.
- **Waits are bounded.** A wait lasts 10 minutes by default, 30 at most, and
  never stops the other thread. A timed-out `delegate_task` goes back to
  automatic delivery.
- **Identity.** Each session Dove starts carries a private session key, so
  a tool call is attributed to the exact thread that made it, even with
  several threads in one folder.
- **Your approvals stay yours.** A child that needs an approval or asks a
  question waits for you in its own thread, exactly like a thread you
  started.

## Compared with t3code's orchestrator

[t3code](https://github.com/pingdotgg/t3code)'s Orchestrator V2 (MIT) set the
bar: `delegate_task` to any provider and model, `task_status`/`task_cancel`,
thread tools, batched wake-ups, no permission escalation, idempotent
retries, worktree isolation. Dove has the same model and adds:

- **Teammates.** Named, briefed personas with persistent notes that any
  agent can run as, and `team_message` for talking to them, not just
  spawning anonymous workers.
- **Waiting that's aware of you.** A child stuck on an approval reports that
  instead of hanging its parent, and reports again when you unblock it.
- **Every agent.** The tools ride on Dove's tool server, so every agent Dove
  hands it to (Claude Code, Codex, OpenCode, Cursor, Pi, Gemini and the
  other ACP agents) gets them, in chat or terminal mode, and any of them can
  hand work to any other.
- **One surface.** Delegated work is ordinary threads in the sidebar and
  under the teammate, with cards that show who asked whom for what.
