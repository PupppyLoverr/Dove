# Notch V3: plan

V3 makes the notch the most capable surface in Keplorayx. You talk to K22, a
Jarvis-style assistant, from anywhere. Ask it something and the notch unfolds
into a live, generated view of what's happening and what to do next. Any
agent's plan appears in the notch for you to approve, edit or reject. All of it
uses the K22 icon pack and the black-island look.

Status: plan only. Nothing here is built. The design goes into Paper first
(paper.design), then into code.

### Decisions so far (from you)

| Question | Decision |
|---|---|
| A call when the app window is closed | **Runs without the window.** The call moves off the Shell into the engine (see §3c) |
| Shortcut | **Reuse the app's Voice Mode shortcut.** When the notch is running, the call shows in the notch |
| Remaining Solar icons | **Drawn in K22 style** for V3 |
| Plan approval in the notch | **Every agent that proposes a plan**, not just K22 |
| Voice default harness and model | **Set in Settings › Voice** (see §3b) |

---

## 0. Where things stand

| Item | State |
|---|---|
| Black island (solid black, glass opt-in, flat cards, shared empty states, Home shortcuts) | Built and checked on screen, **not committed**, in the main checkout on `feat/notch-v2` |
| K22 icon pack, Inter + JetBrains Mono (`689e34d`) | On `feat/ui-v4` only |
| UI v4's notch rework (about 1,440 lines across 16 notch/island files) | On `feat/ui-v4` only |
| Rebrand Dove → Keplorayx (K22) | Uncommitted in the main checkout, by another session; it touches `notch/mod.rs` and `settings.rs` |
| Voice Mode | `voice_call.rs` on the app's Shell, plus `crates/dove-voice`: pipeline, `Conductor`, intents, optional model router. The notch has push-to-talk only (`ToEngine::Dictate`) |
| Generative UI | Already exists: agents call the `show_visual` MCP tool with a Luau script, which returns a `zeron_visual` node tree that `visuals.rs` renders (charts, tables, dashboards, controls) |
| Plans and progress | `AgentEvent::Plan { text }`, `Todos { items }`, `Status`, `ToolDiff` and `PermissionRequested` all flow from every harness through `dove-proto` |

**Step zero is getting onto one base:**
1. Commit the black island.
2. Let the rebrand land.
3. Merge `feat/ui-v4` into `feat/notch-v2`, keeping UI v4's structure and the
   black island's material.

The conflicts will be heavy, mainly in `notch/views/pages.rs`.

---

## 1. Icons: one K22 family

- **Free with the merge:** the K22 pack replaced most SVGs under the same
  names.
- **New names to use:**

  | Notch spot | K22 icon |
  |---|---|
  | Brand mark | `k22-mark` |
  | Agents tab | `cursor-agent` |
  | Thinking | `sparkle` |
  | Tray tab | `tray` |
  | Clipboard items | `clipboard` |
  | Overflow | `more` |
  | New agent / note | `new` |
  | Send | `send` |
  | Voice | `waveform` |
  | Approval preview | `file-diff` |
  | Safe approval | `shield-check` |
  | Projects | `projects` |
  | Model | `model` |
  | Automations | `workflow` |
  | Links | `link` |
  | Spinners | `loader` |
  | Tray in / out | `upload`, `download` |
  | Notch settings | `customize` |
  | Plans | `layers` |
  | Generated views | `artifacts` |
  | Prompt | `prompt` |

- **New K22 drawings (the Solar leftovers),** on K22's 16-grid, at K22's stroke
  weight and corner radius, saved under their existing names so call sites
  don't change:
  - first: `media-play`, `media-pause`, `media-next`, `media-previous`,
    `media-shuffle`, `repeat-play`, `music-note`, `timer`, `alarm`,
    `note-text`, `server`, `git-merge`, `undo`, `microphone-2`, `send-up`;
  - then: `chart`, `code-scan`, `screen-capture`, `highlighter`,
    `box-select`, `bolt`, `chat-line`, `calendar-time`, `clipboard-list`,
    `link-chain`, `eraser`, `hand-shake`, `chevron-down`, `sparkle-agent`.
  - Update `THIRD_PARTY_NOTICES.md` (remove each name from the Solar list) and
    regenerate the Android vectors (`scripts/android/svg2vd.py`).
- **Sizes:** 16 in rows, 18 in the header, 20 on empty-state plates, 28 on the
  call orb. Colour only for meaning.

---

## 2. The look: black island, revamped

- **Tokens:**
  - Type: Inter at 11 / 12 / 13 / 15, and Inter Display SemiBold at 20 / 28
    for the timer, the track title, call captions and generated-view headings.
    JetBrains Mono for code, counts and timers.
  - Surfaces: black, card 6%, wash 8%, hover 13%, on 18%, hairline 10% (menus
    only).
  - Accents: one per meaning, plus `K22` (a brand hue) for live voice only.
  - Radius: island 30, cards 16, rows 12, pills full.
- **Slim capsule:** one live activity with its icon on the left, one short live
  value on the right.
  - Priority: **call** > plan awaiting approval > needs-you > running agents >
    timer > music > pinned note.
- **Expanded header:** icon tabs, then on the right **Ask K22** (a single field
  that is both the command bar and the K22 prompt), **Talk** (`waveform`) and
  overflow.
- **Sizes:** the island grows past today's 468×404 when needed. The canvas
  becomes resizable per state: expanded can go to 720×640 for generated views
  and plans (see §4d). It is still one window that morphs, no new windows.
- **Motion:** keep the springs. New: the "unfold" (a staggered reveal as the
  island grows into a generated view), the call orb (breathing, driven by the
  mic and speaker levels), and the plan step tick-off. Reduced motion falls
  back to fades.

---

## 3. K22 voice: Jarvis mode

### 3a. What it feels like

- **Always one shortcut away.** The voice shortcut (or a long press on the slim
  capsule) gives you the orb straight away, with no app window. You speak and
  K22 answers out loud while the notch shows what it's doing.
- **It sees your work.** K22 knows your threads, agents, PRs, timers, music,
  tray and the foreground app (the Conductor's `Snapshot` today, extended).
  "What's everyone doing?" gives a spoken summary *and* an unfolded status view
  (§4).
- **It acts.** "Start Claude on the auth bug in dove", "reply to the tests
  agent: use the mock", "pause the music", "set a 25 minute timer", "merge
  that PR", "show me the plan". Each one is a checked `Action`, carried out
  through the same paths a click takes.
- **It interrupts politely.** It announces finishes, failures and plans waiting
  on you when you're idle or in a call (`voice_announce`), and stays quiet in
  focus mode.
- **Barge-in:** you can talk over it at any point (already in the pipeline).
- **It hands off.** "Show me" opens the generated view; "open it" opens the
  thread in the app.
- **Safety stays:** destructive actions are never confirmed by voice alone
  (`AwaitClick`). The notch shows the Approve button; you click it.

### 3b. Settings › Voice: default harness and model

New settings, stored in the store like the other `voice_*` keys:

| Key | What it does | Default |
|---|---|---|
| `voice_harness` | Which agent K22 runs on for general questions and new threads (Claude Code, Codex, OpenCode, Cursor, …; installed ones only) | The current default agent |
| `voice_model` | That harness's model (from the harness's model list; "Harness default" first) | Harness default |
| `voice_effort` | Reasoning effort, if the harness supports it | Low (voice wants speed) |
| `voice_project` | Project for new threads started by voice | The last used, or none |
| `voice_persona` | K22's speaking style: concise, chatty or formal (fed to `SetStyle`) | Concise |
| `voice_router` | Intent routing: the built-in rules, the local router (`voice_router_url`), or the voice harness itself | Built-in, then the harness |

- The existing `voice_model` key is the *dictation* model, so the new key is
  `voice_agent_model`; the name above is shorthand.
- The settings UI goes in the Voice section, under "Voice Mode": two pickers
  (agent, model) with the agent's brand mark and the `model` icon.
- The same pickers appear in the notch call page's target chip, as a quick
  override for this call.

### 3c. Architecture: the call leaves the window

The call today is a `VoiceCall` entity owned by the Shell, and its effects go
through `TaskView`/`Shell`. To run without a window:

1. **Split `voice_call.rs`:**
   - `voice_core.rs` (engine side, no views): owns the `Call`, the `Conductor`,
     the snapshot builder and the effect executor.
   - `voice_call.rs` keeps the app's strip and views and subscribes to the
     core.
2. **Effects without a TaskView:**
   - Act on threads through the `Engine` directly, as the headless island host
     already does (`engine.answer`, `start_headless`, `engine.send`,
     `engine.stop`).
   - When the app window is open, keep routing through `TaskView`, so the
     transcript, permission modes and the Approve highlight behave exactly as
     today.
3. **Home of the call:** an `ActiveCall` global on the engine (the island
   `Host` holds it in headless mode, and the Shell reads the same global when
   the window is open). One call at a time, wherever it was started.
4. **K22's own thread:** a general question with no target thread goes to a
   **K22 thread** on `voice_harness`/`voice_agent_model`, created once per
   day (or per call; tune it).
   - It has the Keplorayx MCP tools (including `show_visual` and a new
     `notch_view`, §4), so K22 can drive the notch.
   - The Conductor still handles commands locally ("pause music") without
     asking the model.
5. **Shortcut:** `hotkey.rs` keeps the one global voice shortcut.
   - When the island is connected, it sends `ToIsland::Call` to show the orb,
     and the engine starts the call. Without the island, it behaves as today.

### 3d. IPC (`island/ipc.rs`)

```rust
// island → engine
Call { on: bool, task: Option<TaskId> }
CallMute { on: bool }
CallSay { text: String }               // typed into the call
CallTarget { task: Option<TaskId> }    // switch thread (None = K22)
Ask { id: u64, text: String }          // typed Ask K22 (no voice), see §4

// engine → island
CallState { live, phase, target, caption, partial, level_mic, level_out, elapsed_s, error, awaiting_click }
View { id: u64, view: NotchView }      // a generated view, see §4
ViewPatch { id: u64, patch: ViewPatch }
```

- `phase` mirrors `dove_voice::pipeline::State`.
- Every new field is `#[serde(default)]`, so older engines and notches keep
  working together.
- Levels are throttled to about 20 Hz. No audio crosses the pipe.

### 3e. The call in the notch

- **Slim:** a wide capsule with the `k22-mark`, a live waveform, the state word
  and a red end button.
- **Orb page (expanded):**
  - a large orb whose ring is driven by the mic level while you talk and the
    speaker level while K22 talks;
  - live captions (the last 3–4 lines, with the partial in a fainter colour);
  - the target chip (K22, or a thread with its agent mark) with the agent and
    model override;
  - controls: mute, type, show me, open in app, end.
- **Unfold:** when K22 shows something, the orb shrinks into the header and the
  generated view (§4) fills the island. The captions keep running in a thin
  strip at the bottom.
- **Plans:** if an agent proposes a plan during a call, K22 says "Claude has a
  plan for the auth fix — want to see it?" and the plan view (§5) opens.

---

## 4. Ask K22 → the notch unfolds (generative views)

You ask something by typing in the Ask field or by voice. The island grows,
then K22 builds a **live view** of the answer: what's happening, what needs
you, what to do next. It isn't a chat bubble.

### 4a. Two kinds of view

1. **Built-in "lenses"** (instant, no model): native views the notch draws
   straight from the snapshot:
   - **Status:** every agent as a lane with its current `Status`, `Todos`
     progress, files touched (`ToolDiff`), elapsed time and PR checks.
   - **Needs you:** approvals with diff previews, questions, and plans to
     approve, in one queue with keyboard and voice actions.
   - **Today:** what finished, what failed and what's left, with suggested
     next steps.
   - **One agent:** its plan, checklist, the last reply (markdown), changed
     files and controls.

   Simple asks ("what's running", "what needs me") map to a lens through the
   Conductor's intents, with no model call.
2. **Generated views** (K22 composes them): for anything else ("compare the
   two agents' approaches", "what's left for the release", "show test failures
   by file"). K22 calls an MCP tool:
   - `notch_view(spec)`: a declarative tree of **notch blocks** (below).
     Preferred, because it's fast and needs no script.
   - `show_visual(luau)`: the existing Luau visuals for charts, tables or
     interactive content. The engine runs the script and sends the resulting
     `zeron_visual` node tree; the notch renders it with `visuals.rs`'s
     renderer, re-themed for the island.

### 4b. Notch blocks (the `NotchView` schema)

A small, typed vocabulary, so views look native and stay safe:

- `heading`, `text` (markdown subset), `stat` (value + label + trend),
  `progress` (bar or ring), `list` (rows with icon, title, meta and actions),
  `checklist` (todo items, tickable if allowed), `timeline` (events with
  times), `diff` (file + hunk preview), `agent` (a live agent card bound to a
  `TaskId`, so it updates itself), `plan` (§5), `chart` (via `zeron_visual`),
  `actions` (buttons), `split` (two columns), `section`.
- **Actions** are references to checked commands, never free code:
  - `reply{task,text}`, `approve{task,request}`, `deny`, `stop{task}`,
    `open{task}`, `start{agent,project,prompt}`, `merge{task,pr}`,
    `run_automation{id}`, `timer{mins}`, `ask{text}` (a follow-up question).
  - Destructive actions keep today's rules: a visible button the user clicks,
    never auto-run.
- **Live binding:** blocks can reference snapshot data (`agent: <id>`,
  `todos: <id>`), so the view keeps updating as agents work. No new model
  call is needed for it to stay current.
- **Streaming:** the engine sends a `View` as soon as the first blocks exist
  and then `ViewPatch` updates, so the island unfolds progressively (the
  skeleton first, then filled in).

### 4c. Where views live

- A view is pinned to the island until dismissed. Esc goes back to the tab you
  came from.
- Each view goes into a **Views** history (the `artifacts` icon) in the
  overflow menu: re-open, pin to Home, or "open in app" (rendered in a thread
  side panel with the same blocks).
- A view can be pinned as a **Home widget** (for example "Release status"),
  which stays live through its bindings.

### 4d. Sizing

- The island grows to fit the view, up to 720×640, centred under the notch,
  then scrolls inside.
- The canvas window grows to match: today it's fixed at 520×440, so this needs
  `win.rs` to resize the canvas when a view opens and shrink it after.
- On small displays the maximum is 90% of the display's width and 70% of its
  height.

---

## 5. Plans in the notch: review and approve

- **Source:** every harness's `AgentEvent::Plan { text }`, together with the
  `PermissionRequested` that gates the plan (for example Claude Code's plan
  mode exit), plus `Todos` once work starts.
- **Detection:** the engine pairs a `Plan` with the pending permission request
  that approves it. The snapshot gets
  `plans: Vec<PendingPlan { task, request_id, text, steps, agent, project }>`.
  - `steps` are parsed from the plan's markdown: numbered or bulleted items
    with nested detail.
- **Attention:** a pending plan ranks just under a live call. The slim capsule
  shows the agent mark, "Plan ready" and the step count. A banner offers
  **Review**.
- **Plan view:**
  - the title, the agent and project chips, and a step list;
  - each step is expandable, can be edited inline (the text) and can be
    removed or reordered (drag, or Alt+↑/↓);
  - a "files it expects to touch" summary when the plan names paths.
  - Footer: **Approve**, **Approve with edits** (sends the edited plan back as
    the reply with the approval), **Reject…** (with a reason that goes to the
    agent) and **Open in app**.
- **Voice:**
  - "Read me the plan" gets a spoken summary.
  - "Approve it" approves.
  - "Change step 3 to use the mock" edits the step.
  - Plan approval is not destructive, so voice can approve it. Anything that
    follows still uses its normal permission rules.
- **After approval:** the plan view turns into a **live checklist** bound to
  the agent's `Todos`, ticking off as it works. It's also available as a lens.

---

## 6. More features (ranked)

1. **Ask K22 (typed):** the header field answers with a lens or a generated
   view, no voice needed.
2. **Talk about this:** a call scoped to one agent, from its row or detail.
3. **Voice reply** in agent detail (push-to-talk; `Dictate` exists).
4. **Now stack on Home:** call, plans, needs-you, running, music, timer.
5. **Live widgets on Home** pinned from views (§4c).
6. **Usage meter:** `snap.limits` as a thin bar past 80%, with the reset time.
7. **PR checks on agent rows** (`snap.prs`); merge with an undo window.
8. **Automations** as chips on Home (`RunAutomation` exists).
9. **Quick settings** in the overflow menu: focus mode, mute banners for an
   hour, keep open, glass, the voice agent and model.
10. **Screenshot to tray,** then drag it into an agent's reply or into the Ask
    field ("what's wrong here?").
11. **Proactive nudges** (off by default): K22 notices "tests failing for 10
    minutes" or "agent idle waiting on you" and offers a one-tap fix in the
    slim capsule.

Out of scope for V3: calendar and email integrations, and any cloud service
beyond the user's own configured harnesses.

---

## 7. Phases

| Phase | Work | Done when |
|---|---|---|
| **P0 Base** | Commit the black island; the rebrand lands; merge `feat/ui-v4`; resolve the notch conflicts | One tree builds; the contact sheet shows the black island with K22 icons and Inter |
| **P1 Design** | Paper boards: slim (idle, agent, plan, call), header with Ask, orb page, the unfold, three lenses, the plan review, Home Now and widgets, the voice settings | You've signed off on the boards |
| **P2 Icons + tokens** | §1 swaps, the new K22 drawings, §2 tokens and the canvas resize | No old names left; the Solar list in the notices is down to the brand marks |
| **P3 Voice core** | §3c split (`voice_core.rs`), headless effects, the K22 thread, §3b settings | A call runs with the app window closed; settings pick the agent and model |
| **P4 Voice in the notch** | §3d IPC, §3e UI, the shared shortcut | Start, talk, mute, switch, end from the notch; state mirrored both ways |
| **P5 Plans** | §5 detection, snapshot, plan view, approve / edit / reject, voice | Claude Code plan mode is approved from the notch; the edited plan reaches the agent |
| **P6 Lenses** | §4a built-in lenses, Ask K22 typed | "What needs me" and "what's running" open instantly |
| **P7 Generated views** | `notch_view` MCP tool, block schema and renderer, `show_visual` in the notch, streaming patches, history and pinning | K22 composes a view from a free-form ask; it updates live |
| **P8 Extras** | Features 6–11, motion pass (`-Motion` sheet), polish | Timings within NOTCH-V2 §11.7 |

**Tests at each phase:**
- serde back-compat for the new IPC messages;
- a scripted call through `qa_input()`, asserting the `CallState` sequence;
- the plan step parser on real plan texts;
- the `NotchView` schema, including that unknown blocks are ignored;
- action checking: a destructive action can never run without a click.

**QA:**
- builds go into the warm `target-check`;
- contact sheets on a scratch DOVE_HOME with new hooks:
  `DOVE_NOTCH_PAGE=call|plan|view:<lens>` and demo data for each;
- never use the real `~/.dove`, and never kill a running Keplorayx.

---

## 8. Still open (smaller)

1. **K22 thread lifetime:** one per day, one per call, or one forever with
   compaction?
2. **Generated views off a local model:** should `notch_view` also work through
   the local router (`voice_router_url`) for offline use, with fewer block
   types?
3. **Plan edits:** send the edited plan as a normal reply after approval, or
   reject with "use this plan instead"? This depends on the harness; check
   Claude Code, Codex and OpenCode.
