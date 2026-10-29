# Roadmap v2: Keplorayx, the #1 ADE, at $0 spend

Status: **proposal** (2026-10-29). Replaces the 2026-10-27 roadmap.
Hard constraints from the owner: **$0 cash spend** until revenue pays for it, **free plan forever**, profit > losses on every paid feature, Keplorayx stays **snappy** and the **install stays under 110 MB**.
Related: [NOTCH.md](NOTCH.md), [revamp-v3/PLAN.md](revamp-v3/PLAN.md), [GROWTH.md](GROWTH.md), [PRICING.md](PRICING.md) (updated by §6 here).

## 1. The bet in one line

> **Keplorayx runs any agent, on any machine you own, many at once, and lets you ship the result from your laptop, your phone or your notch. Native, fast, free with your own keys.**

The big players each lock you into one thing: their model, their cloud, or their editor. Keplorayx's moat is the opposite: **every agent × every machine × one native window**. We never pay for compute or tokens, so every paying user is close to pure margin.

## 2. The field (October 2026)

Prices from public pages and aggregators this week; re-check before quoting in launch copy.

| Product | What it is | Price | Where it runs | Where Keplorayx beats it |
| --- | --- | --- | --- | --- |
| **Claude Code** (CLI, desktop, web) | Anthropic's agent; cloud sessions now GA, desktop app with worktrees, remote control | Pro $20, Max $100/$200 | Local, Anthropic cloud, your SSH | One vendor only. Keplorayx runs Claude Code *and* everything else, in one window, on hosts Anthropic doesn't support (WSL, any VPS, Cua Spaces) |
| **Codex** (CLI, app, cloud) | OpenAI's agent, cloud tasks in ChatGPT | Bundled in ChatGPT Free→Pro ($0–$200) | Local, OpenAI cloud | Same lock-in; no cross-agent compare |
| **Devin** | Autonomous cloud engineer | Free, Pro $20, Max $200, Teams $80+$40/seat (ACU-metered) | Cognition's cloud only | Expensive per unit of work, opaque VMs. Keplorayx: your machine, your keys, any agent |
| **Manus** | General autonomous agent with its own VM | ~$19–39/mo entry (unconfirmed), credits | Manus cloud only | Not a dev environment; credits run out. Keplorayx + Cua Spaces gives the same "agent has a computer" on your own account |
| **Cursor / Windsurf / Antigravity** | Editor-first, background agents | ~$20, teams ~$40 | Local + vendor cloud | Editor, not an agent cockpit; heavy Electron |
| **Conductor / worktree runners** | Mac app for parallel Claude/Codex | Free → ~$50/mo | Local Mac | Mac-only, two agents, no remote |
| **Hermes Agent** (Nous) | Open agent with many terminal backends, gateway, skills, memory | Free, OSS | Local, Docker, SSH, Modal, Daytona, Cua… | A single agent, Python, not a GUI ADE. Keplorayx adopts its backend breadth (§4) and runs Hermes as one of its agents |

**What users now expect as table stakes (we must match):** work continues with the laptop closed, start/approve from phone, a VM the agent fully controls, parallel worktrees, PR loop.
**What nobody has (we win on):** cross-agent race + compare, any-host execution with one UI, a native 60–80 MB app, notch/phone/voice as ambient control, $0 platform fee for BYOK.

## 3. Scorecard (CI-enforced where possible)

| Measure | Target | Enforced by |
| --- | --- | --- |
| Installer + installed size | **< 110 MB** (target 85 MB) | New CI size gate on `package.yml` artifacts |
| Binary | < 75 MB | CI gate |
| Cold start → window | < 400 ms release | Perf job (startup trace) |
| Idle CPU / RSS | < 0.5 % / < 80 MB, zero idle frames | Perf job |
| Remote session first token | < 3 s on an already-running host | Integration test against a local SSH/WSL host |
| Download → first agent reply | < 60 s | First-run QA script |

**Size rules:** nothing heavy ships in the installer. Voice models, ffmpeg, Cua images, the headless node for other OSes: all downloaded on demand with pinned SHA-256 (the voice runtime pattern). Every new dependency states its binary cost in the PR. Prefer system tools we can shell out to (`ssh`, `wsl.exe`, `cua`, `docker`) over bundling clients.

## 4. Run anywhere: the Host layer (the headline of v2)

### 4.1 One abstraction

Today every agent is spawned through two places: `dove_harness::process::command` (tokio `Command`, stdio JSON) and `dove_term` (`portable_pty::CommandBuilder`). Both become **host-aware**:

```text
Host = Local | Wsl { distro } | Ssh { alias } | Node { id } | CuaSpace { id }
spawn(host, program, args, cwd)  -> same stdio/PTY handles as today
fs(host)                          -> read/write/list for diffs, worktrees, mentions
git(host)                         -> runs git on the host (worktrees live where code lives)
```

The UI, `AgentEvent` stream, transcript, Team, notch and phone see no difference. A small "where" chip in the composer replaces "Cloud · soon": **This PC · WSL: Ubuntu · ssh: hetzner · Node: home-desktop · Space: dove-sbx-1**.

### 4.2 The targets, cheapest first

| Target | How (all $0 for us) | Size cost | Notes |
| --- | --- | --- | --- |
| **WSL** | `wsl.exe -d <distro> --cd <path> -- <agent>`; paths mapped `C:\…` ⇄ `/mnt/c/…`; or keep repo inside the distro for speed | 0 | Windows devs want Linux toolchains; agents behave better there. Detect distros via `wsl -l -q` |
| **SSH / VPS** | System `ssh` with ControlMaster multiplexing (OpenSSH ships with Windows/macOS/Linux); read `~/.ssh/config` aliases | 0 | Bootstrap installs agent CLIs on the host with the commands `updates.rs` already knows. Keys stay in the user's agent |
| **Node** (any device you own: desktop at home, Mac mini, a VPS) | `dove-node`: a **headless** build of the harness (no GPUI, ~10–15 MB) that dials **out** to the existing relay (CF Worker, `mesh/`). No inbound ports, works behind NAT | separate download | This is "connect another device like Claude": pair with a code/QR, the laptop drives, the node runs. Laptop lid closed → work continues |
| **Cua Space** | Drive Cua through its MCP/CLI (`create_space on=local|aws|gcp|modal`, `space_bash`, `agent_start`, `upload/download`, `open_space_viewer`) | 0 | Gives agents a full desktop (computer use) in a disposable VM; local Spaces are free, cloud Spaces bill the **user's** AWS/GCP/Modal account. This is our "Manus/Devin computer" at $0 to us |
| **Bring your own cloud** | Two routes: (a) any VM reachable by SSH or running `dove-node`; (b) Cua's BYOC (AWS/GCP/Modal) for VMs created on demand with auto-stop | 0 | One-click guides for the free tiers people already have: Oracle Cloud Always Free ARM, GitHub Codespaces free hours, Google Cloud free e2-micro, Fly/Modal trial credits |
| **Keplorayx Cloud (hosted)** | Only after revenue covers it; resell BYOC capacity (Modal/Fly) at cost + 40% | — | Never a loss leader. Until then the composer says "Cloud: your machines" |

### 4.3 What the cloud experience feels like

1. Composer "where" chip → **Add a machine**: WSL (auto-detected), SSH host (from config), Node (pair code), Cua Space (local or your cloud), each with a health check.
2. Pick a machine, type the task. Worktree is created **on that host**; diffs stream back as text, never the whole repo.
3. Close the laptop. The node/Space keeps going; the phone and notch get "needs you / done" from the relay.
4. Open Keplorayx anywhere, the thread reattaches (sessions already persist; the relay carries the live stream).
5. Race across machines: same prompt to Claude on WSL, Codex on the VPS, Gemini in a Space; compare (§5).

### 4.4 Lessons taken from Hermes Agent

See §9 for the full analysis of `Downloads/cu-research/hermes-agent`. The adopted ideas: pluggable terminal backends behind one interface, per-backend config instead of per-agent, persistent remote sessions with explicit lifecycle (create → reuse → idle-stop → delete), a messaging gateway so agents can be reached from chat apps, and scheduled (cron) runs on an always-on host.

## 5. Product pillars (kept from v1, re-ordered for revenue)

| # | Pillar | Why it pays | Size |
| --- | --- | --- | --- |
| P1 | **Host layer: WSL + SSH** (§4) | Headline feature, free, nobody else does any-agent × any-host | M |
| P2 | **Race and compare**: one prompt → N agents/hosts → side-by-side diffs, tests, cost, time → merge winner | The launch video; drives installs | L |
| P3 | **Trust and first run**: self-update, first-run wizard, free signing (§7) | Without it, launch fails | M |
| P4 | **dove-node + relay pairing** (laptop-lid-closed) | First Pro hook (hosted relay limits) | L |
| P5 | **Cua Spaces + BYOC** | "Agent gets a computer", matches Manus/Devin at $0 | M |
| P6 | **Review loop**: inline comments → agent, per-turn checkpoints/rewind, per-worktree run/preview, PR CI feedback | Retention | L |
| P7 | **Ambient**: notch, phone, voice, push | Pro conversion (push is Pro) | ongoing (NOTCH.md) |
| P8 | **Schedules and automations**: cron runs on a node/Space, triggered by GitHub events | Pro/Team; sticky | M |
| P9 | **Context**: one rules file synced to CLAUDE.md/AGENTS.md/GEMINI.md, memory, handoff between agents | Retention | S–M |
| P10 | **Catalog**: skills, MCP servers, teammate templates, share-my-setup | Community growth | M |

## 6. Money: free plan, paid plans, and why it's profitable

### 6.1 Principles (unchanged and tightened)

1. **BYOK and local are free forever.** All agents, all hosts the user owns (WSL, SSH, Node via self-hosted relay, local Cua Spaces), race/compare, review loop.
2. **We charge only for things that run on our infrastructure or save real time across devices/people.** Our infra = the Cloudflare relay (free tier covers launch volume; paid Workers is $5/mo and only needed once revenue exists).
3. **Every paid feature has positive unit margin from day one.** Nothing we sell costs us compute.
4. Gate on the relay/license server, never by crippling the local binary.

### 6.2 Plans

| | **Free** | **Pro** $12/mo ($10 yearly) | **Team** $24/user ($20 yearly) | **Enterprise** |
| --- | --- | --- | --- | --- |
| Agents, worktrees, race/compare, review loop | ✓ | ✓ | ✓ | ✓ |
| Hosts: local, WSL, SSH, local Cua Spaces | ✓ | ✓ | ✓ | ✓ |
| Paired nodes via Keplorayx relay | 1 node | Unlimited | Unlimited | Unlimited |
| Phone remote via Keplorayx relay | 1 phone | Unlimited | Unlimited | Unlimited |
| BYOC cloud Spaces (your AWS/GCP/Modal) | Manual | Auto-sleep, budgets, fleet view | + shared pool | + policy |
| Push notifications, scheduled/cron runs | — | ✓ | ✓ | ✓ |
| Session history on relay | 24 h | 90 days | 1 year | Custom |
| Settings/templates sync | — | ✓ | ✓ | ✓ |
| Shared projects, hand off a thread, seats | — | — | ✓ | ✓ |
| SSO, audit, self-hosted relay support | — | — | — | ✓ |

Self-hosting the relay removes every limit (OSS promise). Students and OSS maintainers: Pro free.

### 6.3 $0 stack to collect money

| Need | $0 option | Cost when it scales |
| --- | --- | --- |
| Payments, tax, invoices | **Polar.sh** or **Lemon Squeezy** (merchant of record, no monthly fee) | ~4–5 % + fixed fee per charge |
| License check | Signed license token issued by a CF Worker on payment webhook; verified offline by Keplorayx | $0 on CF free tier |
| Relay | Existing Cloudflare Worker + Durable Objects, free tier | Workers Paid $5/mo, pay only after first revenue |
| Website, docs, downloads | GitHub Pages / CF Pages + GitHub Releases | $0 |
| Crash reports, activation ping | CF Worker + D1 (free tier), opt-in | $0 |
| Email | CF Email Routing (inbound), Resend free tier (outbound) | $0 at launch |

### 6.4 Rough P&L (GROWTH.md funnel, 2 % of activated users on Pro by week 12)

| | Week 12 | Month 12 (if growth holds ~10 %/mo after) |
| --- | --- | --- |
| Pro users | ~1,900 | ~6,000 |
| MRR | ~$23k | ~$70k (+ Team seats) |
| Payment fees (~5 %) | ~$1.2k | ~$3.5k |
| Infra | $0–5 | ~$50–300 |
| Margin | ~95 % | ~95 % |

These are planning numbers, not forecasts. They only hold if activation and conversion hit GROWTH.md's ratios. The one cost that grows is the owner's time; keep support to docs + Discord.

## 7. Trust at $0

| Item | $0 route | Paid upgrade (from revenue) |
| --- | --- | --- |
| Windows signing | **SignPath Foundation** free code signing for OSS projects (apply now; needs a public repo and CI builds) | Azure Trusted Signing (~$10/mo) |
| macOS | Ship via Homebrew cask + documented "right-click → Open"; notarization needs the $99/yr Apple program, buy it from the first month's revenue | Developer ID + notarization |
| Linux | AppImage + .deb/.rpm, GPG-signed checksums | — |
| Self-update | Signed manifest (minisign/ed25519 key held by the owner), GitHub Releases as CDN, "restart to update" pill | Delta updates |

## 8. Order of work

| Phase | Contents | Exit criteria |
| --- | --- | --- |
| **0. Today** (this session) | Roadmap v2; Host enum + WSL/SSH spawn wrapping in `dove-harness`/`dove-term` behind a setting; CI size gate | `cargo check` green; WSL agent run works by hand; CI fails above 110 MB |
| **1. Ship-ready** (wk 1–2) | P3 (self-update, wizard, SignPath), composer "where" chip, SSH bootstrap, perf gate | Signed Windows build auto-updates; Claude Code runs over SSH from the UI |
| **2. The wedge** (wk 3–4) | P2 race/compare across hosts, Polar checkout + license, notch N1 | 30-second "three agents, three machines" video is real |
| **3. Laptop closed** (wk 5–7) | P4 dove-node + pairing, push, P5 Cua Spaces + BYOC, P8 schedules | Close laptop mid-task, approve from phone, merge from Keplorayx |
| **4. Close the loop** (wk 8–10) | P6 review loop, P9 context, Team plan | Team plan sold to first 10 teams |
| **5. Ecosystem** (wk 11+) | P10 catalog, Enterprise, hosted Keplorayx Cloud only if margin-positive | Month-over-month profit |

## 9. Hermes Agent analysis (`Downloads/cu-research/hermes-agent`)

**Cua: Hermes does not integrate Cua Spaces.** Its only Cua use is the local `cua-driver` binary for background computer use (`tools/computer_use/cua_backend*.py`, `plugins/computer_use/cua/`, installed by `hermes computer-use install`). Its "Bot Desktop" is a headless Xfce/TigerVNC screen on the gateway host or inside the Docker/SSH sandbox, streamed to Hermes Desktop for human takeover (logins, 2FA). **Keplorayx driving Cua Spaces directly (local, AWS, GCP, Hetzner, Modal) is a feature Hermes does not have.**

**Execution backends** (`tools/environments/`, chosen by `terminal.backend`): local, ssh (ControlMaster, except on Windows), docker (egress control, cwd mount off by default), modal and daytona and vercel_sandbox (user's own key, snapshot/resume), managed_modal (paid via Nous), singularity as a plugin. `BaseEnvironment` spawns a fresh `bash -c` per command and re-sources a captured shell snapshot; `file_sync.py` pushes files to remote sandboxes; `lifetime_seconds` defaults to 300.

**Worth copying into Keplorayx:**

| Hermes feature | Keplorayx version |
| --- | --- |
| Pluggable terminal backends, config per backend | `dove_harness::host::Host` (done, §4) + Docker and Modal later as more variants |
| Persistent sandboxes: snapshot on stop, resume on reuse | Cua checkpoints/fork (`cua sb checkpoint`, `fork`): sub-second warm Spaces per task |
| Messaging gateway (Telegram, Discord, Slack, WhatsApp, Signal, email, ntfy…) | Pro: reach your agents from chat apps through the relay; ntfy/Telegram first (free APIs) |
| Natural-language cron jobs delivered to any platform | P8 schedules on a node or Space |
| Skills that the agent writes and improves itself (agentskills.io) | Keplorayx already has Teach; add self-improvement + catalog (P10) |
| Session FTS5 search + summaries, memory plugins | P9 context |
| Tool-result size caps that spill to disk | Adopt for every agent's tool output in the transcript (perf + RSS) |
| 40 model-provider plugins | Not needed: Keplorayx runs agent CLIs that bring their own providers |

Hermes ships an installer on this machine (`Downloads/Hermes-Setup.exe`); add **Hermes as a Keplorayx agent** through ACP/external so its users get Keplorayx's UI.

## 10. Decisions (2026-10-29)

| Question | Decision |
| --- | --- |
| What to build first | Host layer: WSL + SSH + Cua Spaces, testable against the owner's local Spaces |
| Payments | **Polar.sh** (merchant of record, no monthly fee) |
| Hosted Keplorayx Cloud | **Only after revenue**, priced at cost + 40 %; until then, the user's own machines and cloud accounts |
| Pricing | Keep **Free / Pro $12 ($10 yearly) / Team $24 ($20 yearly)** |

## 11. Status of the Host layer (built 2026-10-29)

- `crates/dove-harness/src/host.rs`: `Host { Local, Wsl, Ssh, CuaSpace }`, `wrap()` to a local invocation, WSL path mapping, POSIX quoting, discovery (`wsl -l -q`, `~/.ssh/config`, `cua sandbox ls --json`), serde for settings. `process::command_on` spawns an agent on a host.
- Verified by hand: WSL runs commands (`wsl.exe -d <distro> --cd … -- …`); a Cua Space round-trips JSON over stdio with `cua sandbox exec -i --cwd /tmp local:space-1 cat`.
- **Composer chip (built):** the device chip lists This PC, WSL distros, `~/.ssh/config` hosts and Cua Spaces (found off the UI thread when the menu opens). A new thread saves its host (`host.<task>` setting); resume and fork reuse it; an open thread shows its host read-only. "Cloud · soon" is gone.
- **Every agent goes remote the same way:** for a non-local host, `dove_harness::spawn` swaps `spec.program` for a launcher, a hard link to `dove.exe` at `%TEMP%/dove-hosts/<host>/<agent>.exe` with `host.json` beside it. `main` checks `host::run_launcher()` first; under that name Keplorayx hops through `wsl.exe`/`ssh`/`cua sandbox exec -i` with inherited stdio and exits with the agent's code, never loading the UI. Zeron's drivers (Claude Code, Codex, OpenCode, Pi, Devin, Grok, Hermes) and Keplorayx's ACP agents need no changes.
- **Cua Spaces:** the launcher resumes a paused Space before `exec` (`exec` refuses a paused one; `resume` fails on a running one, so it checks `info` first). The earlier `--embedded` theory was wrong: the Space was simply paused.
- **Cua Spaces for agents (built), the way Cua documents it:** Settings → Integrations has a one-click **Cua Spaces** entry, `{command: cua, args: [mcp]}`.
  - Like every catalog server, it reaches every agent through Keplorayx's MCP server as `cua__*` tools: `list_spaces`, `create_space`, `space_bash`, `agent_start`, computer use through `call_tool`, and the rest.
  - Agents get a short hint: work in a Space and only delete Spaces they created.
  - Checked against `cua` 0.9.0 on this PC: the server answers the MCP handshake and lists 156 tools.
  - The "run the whole agent in a Space" host above still uses `cua sandbox exec -i`. An agent's JSON-over-stdio needs a live stream, and MCP's `space_bash` returns only when the command finishes.
- **End-to-end test (2026-10-29), OpenCode + `opencode/deepseek-v4.1-flash-free`:**
  - **Agent in a Space:** Keplorayx's harness ran OpenCode inside `local:space-1`. Its shell tool reported the Space's hostname and `/home/cua/Temp`, in 28 s.
    - This needed two launcher additions. Drivers that serve over HTTP (`opencode serve --port P`) get that port forwarded for the session: `cua sandbox port-forward` for a Space, `ssh -L` for SSH; WSL2 forwards loopback by itself. `OPENCODE_*` variables travel with the command.
    - It also needed a `dove.remoteDirectory` option, so the vendored OpenCode driver scopes the session to the folder as the host sees it.
  - **Agent on this PC driving Spaces:** the same model, through Keplorayx's MCP server with the Cua Spaces integration on, called `cua__list_spaces` and `cua__space_bash` (`hostname && whoami` in `space-1`).
    - The 156 Cua tools add about 32k tokens to every session. A narrower `--permissions` set is worth offering.
  - Reproduce with `DOVE_HOST` / `DOVE_TOOL_SERVER` on `crates/dove-harness/examples/smoke.rs`.
- **Host prep (built):** before the agent starts, the launcher runs one short script on the host, with its output captured so nothing reaches the agent's JSON stdout.
  - A missing `<home>/<folder>` is cloned from the local checkout's `origin`, on the current branch when the remote has it, or the default branch with a note when it doesn't.
  - If the agent isn't on the host's `PATH`, the launcher stops with exit 127 and prints the install command (`npm install -g @anthropic-ai/claude-code` and others).
  - Verified in a local Cua Space (`space-1`) and under a local shell.
- **CI size gate (built):** `scripts/size_gate.sh` runs in all three `package.yml` jobs.
  - It fails when any installer, archive or the unpacked Windows folder is over 110 MB.
  - It warns when the release binary is over 75 MB.
- **Known limits of this slice:**
  - SSH and Cua run in `<home>/<folder name>`. The first run clones `origin`, so local commits that aren't pushed are not there; push first. Worktrees are still created locally. WSL uses the Windows path (`/mnt/c/…`), so it works with no setup.
  - Cursor ignores the launcher (it runs Zeron's SDK shim).
  - The injected `dove` MCP tool server points at a Windows path. It works from WSL via interop, not over SSH or in a Space.
  - The agent must be installed locally (its file name picks the remote command) and also on the host.
- **Next:**
  - Sync unpushed work to the host (push to a bare remote on the host, or `cua sb cp`).
  - A one-click "install agent on host" button that runs the printed command.
  - PTY panes (`tty`).
  - `dove-node`.
