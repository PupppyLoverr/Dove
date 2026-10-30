# Environments: every agent on every machine

Status: **plan** (2026-10-29). Implements [ROADMAP.md](ROADMAP.md) §4 (Host layer), P1, P4, P5 and P8, and supersedes its §9 Hermes notes where they disagree (§8 below).
Inputs: the Host layer already in `crates/dove-harness/src/host.rs`; a survey of `NousResearch/hermes-agent` at `ef75bc7` (2026-10-28) and `trycua/hermes-cua-spaces` at `82bb8d3`; vendor docs for 21 sandbox providers (links in §9).
Constraints carried over: $0 spend for us, BYOK, installer < 110 MB, nothing heavy bundled, zero idle cost.

## 1. Goal

From one Keplorayx window, a user can start any agent (Claude Code, Codex, Gemini, OpenCode, Hermes…) on:

- **this machine**, in a WSL distro, or in a local Docker / Apptainer / microVM sandbox;
- **any machine they own**: a VPS over SSH, or a paired device (their Mac, a home desktop) running the headless `dove-node`, reached through the relay with no open ports;
- **any cloud sandbox they bring a key for**: E2B, Daytona, Modal, Vercel Sandbox, Boat, InstaBox, Fly Sprites, Runloop, Morph, Codespaces, Coder, Ona, Northflank, Blaxel, Kubernetes, and Cua Spaces (local or the user's own AWS/GCP/Modal).

The transcript, approvals, diff, browser and team tools, phone, notch and voice behave the same wherever the agent runs. Hermes Agent has the widest backend list of any open agent; this plan matches every backend it has (§3) and adds what it lacks: other people's agents, a native GUI, paired devices, PTY panes on remote hosts, worktrees on the host and a cross-host race.

## 2. Where we are today

What works (ROADMAP §11):

- `Host { Local, Wsl, Ssh, CuaSpace }`. `Host::wrap` turns `program args` into a local `wsl.exe` / `ssh` / `cua sandbox exec -i` invocation.
- **Launcher hop.** For a non-local host, `dove_harness::spawn` replaces the agent binary with a hard link to `dove` under `dove-hosts/<host>/<agent>`. `main` calls `host::run_launcher()` first, which runs `prepare` (clone `origin` if the folder is missing, check the agent is on `PATH`), forwards an `opencode serve` port, then hops with inherited stdio. No driver changed.
- **Composer "where" chip** (`apps/dove/src/composer/mod.rs:1327`). It lists This PC, WSL distros, `~/.ssh/config` aliases and ready Cua Spaces. A thread stores its host in `host.<task>` and keeps it on resume and fork.
- **Cua Spaces MCP integration** (156 `cua__*` tools) in Settings → Integrations.

Gaps this plan closes:

| Gap | Effect today |
| --- | --- |
| No "Add a machine" flow, no health check | Users must hand-edit `~/.ssh/config` or install `cua` first; failures show up only when a run starts |
| Code reaches remotes only by cloning `origin` | Unpushed commits and uncommitted edits are missing on SSH and Cua hosts |
| Worktrees are always local | Two remote agents on one repo share a folder; local diffs don't show remote edits |
| The `dove` MCP server points at a local path | Browser, team, computer-use and `show_ui` tools fail over SSH and in Spaces |
| A dropped connection kills the agent | Sleeping the laptop or losing Wi-Fi ends the remote run |
| No PTY panes remotely (`tty` unused) | Terminal mode doesn't work on remote hosts |
| Ports: only `opencode serve` | Dev-server previews and remote Chromium aren't reachable |
| No cloud provider keys, no lifecycle | No create, stop, resume, delete or cost view for sandboxes |
| `mesh/crates/proto/src/cloud.rs` and `mesh/crates/rpc` define cloud-seat types and RPCs (Railway, Boat, SSH-Docker, GitHub) | Wire types only; no implementation exists in this tree |

## 3. The environment list

"Route" is how stdio reaches the agent (§4.3). Every row is BYOK or free; we never pay for compute.

| # | Environment | Hermes has it? | Route | Auth the user brings | Phase |
| --- | --- | --- | --- | --- | --- |
| 1 | Local | ✓ | direct | — | built |
| 2 | WSL distro | ✓ (plugin) | CLI `wsl.exe` | — | built |
| 3 | SSH / any VPS | ✓ | SSH | ssh config / agent | built → hardened in E1 |
| 4 | Cua Space (local, `host:` machine, your AWS/GCP/Modal) | ✓ (Cua's plugin) | CLI `cua sandbox exec -i` | `cua` login / own cloud | built → lifecycle in E3 |
| 5 | **Paired device (`dove-node`)**: your Mac, home PC, VPS behind NAT | ✗ (Hermes has remote gateways, no pairing) | relay stream | pair code / QR | E2 |
| 6 | Docker (local or remote engine) | ✓ | CLI `docker exec -i` | — | E1 |
| 7 | WSL Containers (`wslc`) | ✓ (plugin) | CLI | — | E4 |
| 8 | Apptainer / Singularity (incl. HPC over SSH) | ✓ (plugin) | CLI, or SSH + CLI | — | E4 |
| 9 | Bubblewrap (Linux, per-command jail) | ✓ (plugin) | CLI `bwrap` | — | E4 |
| 10 | smolmachines microVM (macOS ARM, Linux KVM) | ✓ (plugin) | CLI | — | E4 |
| 11 | **E2B** | ✗ | native (Connect-RPC process stream), or SSH via a template with sshd | `E2B_API_KEY` | E3 |
| 12 | **Daytona** | ✓ | REST create → SSH token | `DAYTONA_API_KEY` | E3 |
| 13 | **Boat** | ✗ | REST create → `POST /sandboxes/{id}/sshkey` → SSH | `BOAT_API_KEY` | E3 |
| 14 | **InstaBox** | ✗ | REST create → SSH gateway `ssh.instabox.sh` | `ibk_…` key | E3 |
| 15 | Modal Sandboxes | ✓ | native exec stream (gRPC, needs a sidecar) | `MODAL_TOKEN_ID/SECRET` | E4 |
| 16 | Vercel Sandbox | ✓ | CLI `sandbox exec -i` | Vercel token + project/team | E4 |
| 17 | Fly Sprites | ✗ | native WebSocket exec, reattachable | `SPRITES_TOKEN` | E3 |
| 18 | Fly Machines | ✗ | REST create → SSH (own sshd) | `FLY_API_TOKEN` | E4 |
| 19 | Runloop | ✗ | REST create → SSH (key + TLS proxy) | `RUNLOOP_API_KEY` | E4 |
| 20 | Morph Cloud | ✗ | REST create → SSH | `MORPH_API_KEY` | E4 |
| 21 | GitHub Codespaces | ✗ | `gh codespace ssh --config` → SSH | GitHub login | E3 |
| 22 | Coder | ✓ (plugin) | `coder config-ssh` → SSH | `CODER_URL` + token | E3 |
| 23 | Gitpod / Ona | ✗ | `ona environment ssh-config` → SSH | `ONA_TOKEN` | E4 |
| 24 | Northflank | ✗ | native WebSocket exec | API token | E5 |
| 25 | Blaxel | ✗ | native REST process + stdin | `BL_API_KEY` | E5 |
| 26 | Kubernetes pod | ✗ | native exec/attach (WebSocket) | kubeconfig | E5 |
| 27 | DeepInfra sandbox | ✓ (plugin) | native | `DEEPINFRA_API_KEY` | E5 |
| 28 | Cloudflare Sandbox | ✓ (plugin) | native WebSocket PTY via the user's Worker | own Worker URL + token | E5 |
| 29 | Railway trial VM | ✗ (in mesh `CloudSeatKind`) | SSH (`ssh railway.new`) | SSH key | E5 |
| 30 | Hermes remote gateway | n/a | not an environment: run **Hermes as an agent** on any host above | — | — |

Deliberately left out: **Nous managed Modal** (paid through Nous, not BYOK) and **Cua's hosted cloud**, which shut down on 2026-10-18 according to Cua's cloud guide (local Spaces and BYOC are unaffected; verify before launch copy). Together / CodeSandbox SDK stays out until it is self-serve.

**Why so many rows are cheap:** 12 of the cloud providers expose SSH once a box exists. Each of those is one REST call (create, then fetch credentials) on top of a single hardened SSH transport.

## 4. Architecture

### 4.1 From an enum to providers

`Host` stays the serialized value a thread stores, so existing `host.<task>` settings keep working. It gains variants:

```rust
pub enum Host {
    Local,
    Wsl { distro: Option<String> },
    Ssh { alias: String },
    CuaSpace { name: String },
    Docker { target: DockerTarget },            // container name or image + engine (local or ssh://)
    Node { id: String },                        // a paired dove-node
    Sandbox { provider: String, id: String },   // any cloud provider in §3, by provider id
}
```

Everything behind `Sandbox` goes through one trait in a new module `crates/dove-harness/src/hosts/` (one file per provider, so a provider costs a few KB and no new crates):

```rust
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;                      // "e2b", "daytona", "boat", …
    fn label(&self) -> &'static str;
    fn policy(&self) -> Policy;                        // see below
    fn probe(&self, keys: &Keys) -> Probe;             // < 2 s, no side effects: Ready | NeedsSetup(why) | Unavailable(why)
    fn list(&self, keys: &Keys) -> Result<Vec<Box_>>;  // existing boxes, with state and cost hints
    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<Box_>;   // image/template, cpu, mem, ttl, repo
    fn wake(&self, keys: &Keys, id: &str) -> Result<()>;              // resume / start; no-op when running
    fn sleep(&self, keys: &Keys, id: &str) -> Result<()>;             // pause / snapshot / stop
    fn delete(&self, keys: &Keys, id: &str) -> Result<()>;
    fn connect(&self, keys: &Keys, id: &str) -> Result<Endpoint>;     // how stdio gets there (4.3)
}

pub struct Policy {
    pub is_remote: bool,          // no host-OS hints, files synced (4.4)
    pub is_container: bool,       // isolated: approvals may relax (§6)
    pub strip_env: &'static [&'static str], // vendor keys never reach the agent's env
    pub ports: PortRoute,         // Ssh | Provider (public URL) | Relay
    pub desktop: bool,            // VNC/computer use available
    pub max_lifetime: Option<Duration>,     // shown in the UI (Modal/Vercel 24 h, E2B Hobby 1 h)
}
```

`probe`, `policy` and `strip_env` are copied from Hermes's `TerminalEnvironmentProvider` (`agent/terminal_env_provider.py`), which drives its picker and env scrubbing from the same flags. `wake` before connect generalizes today's `wake_space`.

### 4.2 The launcher stays the single seam

The launcher hop is the reason no driver changed, so every new environment plugs in there:

```text
driver ─spawns─▶ dove-hosts/<host>/<agent>  (= dove binary)
                   run_launcher():
                     provider.wake(id)                 ┐ silent, stderr only
                     endpoint = provider.connect(id)   │
                     sync::ensure(endpoint, repo)      │ 4.4
                     relay::start_or_attach(endpoint)  ┘ 4.5
                     bridge stdio ⇄ endpoint  (exec for Ssh/Cli, in-process for Native)
```

- `Endpoint::Ssh { host, user, port, identity, proxy_command, known_host }`: the launcher writes a private `ssh_config` fragment under `~/.dove/hosts/ssh/` and runs system `ssh -F` (zero size, ControlMaster where supported). This covers Daytona, Boat, InstaBox, Runloop, Morph, Fly Machines, Codespaces, Coder, Ona and Railway.
- `Endpoint::Cli { program, args }`: the existing `wrap` path (`wsl.exe`, `cua`, `docker exec -i`, `apptainer exec`, `bwrap`, `sandbox exec -i`, `smolvm`).
- `Endpoint::Native(Box<dyn Stream>)`: the launcher itself pumps its stdin/stdout to a WebSocket or HTTP stream (Sprites, E2B, Northflank, Blaxel, Kubernetes, Cloudflare, `dove-node` over the relay). It uses `reqwest` (already in `dove-harness`) plus a WebSocket client shared with `mesh`. The bridge sends plain bytes; never a PTY for JSON stdio.

### 4.3 Keys

- Stored in `~/.dove/cloud-keys.json` (0600), the convention `mesh/crates/proto/src/cloud.rs` already documents, with an opt-in OS keychain backend later. Never in `dove.db`, settings JSON, transcripts or synced CRDT documents.
- Keys reach providers only from the launcher or the app's own process; `Policy::strip_env` removes them from the agent's environment, so an agent can't read its own sandbox's key.
- Settings → **Machines → Cloud providers**: one row per provider with key field, "Test" (calls `probe`), free-tier note and a link to the provider's key page.

### 4.4 Code on the host: sync and worktrees

Replaces "clone `origin` and hope it's pushed":

1. **Bundle push.** Before the first run, `git bundle create - <branch> --not <remote-known>` streams over the endpoint into a bare repo at `~/.dove/repos/<project-id>.git` on the host; later runs send only new commits. Uncommitted changes go as a patch on top (`git diff --binary HEAD`), applied in the worktree. Works on every route, needs only `git` on the host, and keeps working when `origin` is private or unreachable from the sandbox.
2. **Worktree on the host.** `dove_git` gains `Repo::on(endpoint)`, which runs `git` through the endpoint, so `create_worktree_from`, status, diff and history work remotely. Worktrees live at `~/.dove/worktrees/<task>` on the host, next to the bare repo.
3. **Diffs come back as text.** The side panel's diff, history and file viewer call `git diff` / `git show` / `cat` through the endpoint and are capped like local reads. The repository is never copied back.
4. **Bring it home.** A thread's "Apply locally" fetches the worktree branch back with a reverse bundle, so a remote result becomes a local branch to merge or open a PR from.

### 4.5 Sessions that survive the connection: headless Keplorayx on the host

E2B, Vercel and Sprites warn that streams drop while the sandbox keeps running; laptops sleep. So the agent should not be a child of the SSH connection.

**Decision (2026-10-29): there is no separate `dove-node` download.** The desktop binary itself runs headless. `dove node …` is a subcommand dispatched in `main` before any GPUI code runs, the same way `host::run_launcher()` and `--mcp-browser` already are. In this document, "`dove-node`" means "the `dove` binary started as `dove node`".

- **Subcommands:** `dove node serve` (paired device, §5), `dove node attach <task> -- <agent argv>` (run or reattach an agent), `dove node mcp` (remote tool bridge, §4.6), `dove node pipe <addr>` (port relay, §4.7), `dove node pair` (print code + QR).
- **Getting it onto a host:** when the host's OS/arch matches this machine, the launcher copies its own binary (`~/.dove/bin/dove`, refreshed when the version changes). Otherwise it fetches the matching release artifact of the **same version** from GitHub Releases on the host itself (`curl` + pinned SHA-256 from the signed manifest), so nothing large crosses the user's uplink.
- **Headless Linux servers:** GPUI must not need a display at load time. E0 checks `ldd` on the release binary; any X11/Wayland/Vulkan/xkbcommon library linked at load time is switched to `dlopen` at window creation, or, if that is impractical in the vendored GPUI, `dove node` re-execs nothing and the installer note lists the two small packages (`libxkbcommon0`, `libfontconfig1`) a headless server needs. The release size gate (110 MB) is unchanged because no new artifact ships.
- **Attach:** starts the agent under the headless process if absent, otherwise reattaches and replays output since the last acknowledged byte offset. Agent stdio is logged to `~/.dove/sessions/<task>.out` on the host (capped, rotated).
- Without the binary on the host (a locked-down box), fall back to running the agent directly; the UI marks the thread "won't survive disconnects".
- The same subcommand family is the **paired device** of §5 and the **remote tool bridge** of §4.6.

### 4.6 Keplorayx's tools on remote hosts

The `dove` MCP server is `dove --mcp-browser`, which forwards `{token, cwd, session, tool, args}` lines to the app's loopback control port (ARCHITECTURE.md, Browser control path). Remotely:

- The agent's MCP config points at `dove-node mcp` on the host instead of a local path.
- `dove-node mcp` forwards the same JSON lines back over the endpoint: an `ssh -R` reverse forward of the control port on SSH routes, the relay stream for nodes and native routes. The token stays in the env as today; `cwd` is mapped back to the thread so routing by folder still works.
- Result: browser, team, computer use, `show_ui`, Composio and Paper tools work from WSL, SSH, Spaces, nodes and sandboxes with no per-tool changes.

### 4.7 Ports, previews and desktops

- **Generic forward over exec** (Hermes `tools/environments/streams.py:142-274`): the launcher listens on a local port; each accepted connection opens one exec stream running a tiny TCP relay (`dove-node pipe 127.0.0.1:<p>`, or `nc`/`socat`/`python3` when no node). This works on every route that can exec and replaces today's `opencode serve` special case.
- **Provider preview URLs** where they exist (E2B, Daytona, Boat `host`, InstaBox `port publish`, Morph `expose-http`, Codespaces): offered as "Open public URL" with a privacy warning, never by default.
- **Remote desktops:** Spaces, E2B Desktop, Daytona VNC, Boat `desktop` and InstaBox's desktop template expose VNC or a viewer URL. The side panel gains a "Desktop" tab (viewer in the existing web view) with Take over / Hand back, which already exists for local computer use.

### 4.8 PTY panes

Terminal mode on a remote host passes `tty: true` (`ssh -tt`, `cua … -t`, WebSocket PTY on native routes, `dove-node pty` on nodes). `dove-term` only needs the local invocation, which `Host::wrap` already returns. Resize events travel as a control message on native routes.

## 5. Paired devices ("connect your Mac")

The flow the owner asked for: open Keplorayx on the laptop, connect a Mac (or any machine), and agents started from the laptop run on the Mac.

1. On the Mac: install Keplorayx as usual and choose **Settings → Machines → Use this computer as a machine**, or on a server run `dove node pair` (same binary, headless). It shows a QR code and a 6-word code.
2. On the laptop: chip → **Add a machine → Pair a device**, scan the QR (phone app) or type the code. **Decision: pairing requires the Woozlit account** on both sides (already used by the mobile remote), so the edge scopes the rooms; the code only binds this machine to this account. No LAN-only, account-free pairing.
3. `dove-node` dials **out** to the Cloudflare edge (`DeviceRoom` per device, already in the mesh design): no inbound ports, works behind NAT and on hotel Wi-Fi. A LAN/Tailscale direct path is tried first when both sides see each other, to keep latency down and relay traffic at zero.
4. The node advertises: OS, arch, installed agents (it runs `detect.rs`), projects found under chosen roots, free disk, battery/AC. The laptop's chip shows "Node: kamal-mac · 3 agents".
5. Runs execute on the node exactly as in §4.5; the laptop, phone and notch all watch the same session through the relay. Closing the laptop doesn't stop anything.
6. Security: the node runs agents only for its paired account; every command frame is signed by the account token; a node can be set to **ask on the node** before starting a run; Settings → Machines → Revoke unpairs instantly. Pricing per ROADMAP §6.2: 1 paired node free, unlimited on Pro; self-hosted relay removes limits.

A paired Windows PC exposes its own WSL distros and Docker as sub-hosts ("Node: home-pc › WSL: Ubuntu"), so environments compose: any environment in §3 can sit behind a node.

## 6. UI

- **Composer chip** grows sections: This machine · Paired devices · SSH · Containers · Cloud (per provider, with "+ New box") · **Add a machine…**. Each row shows probe status (ready / asleep / needs setup), and for cloud boxes a lifetime or idle timer.
- **Add a machine** wizard: WSL (detected) · SSH (pick from config or paste `user@host`, key test, "write to ~/.ssh/config") · Pair a device · Docker (image picker; default `ubuntu` + agent install) · Cloud provider (paste key → probe → create box with sensible defaults: smallest size, auto-sleep 15 min, TTL) · Cua Space.
- **Settings → Machines**: fleet view (all hosts, state, agents installed, running threads, last probe, sleep/wake/delete, monthly cost estimate where the provider exposes it), keys, default host per project.
- **Install agent on host** button when `prepare` exits 127 (ROADMAP §11 "Next"), running the printed command through the endpoint with output in a sheet.
- **Thread header** shows the host chip, connection state (live / reconnecting / detached-but-running) and "Apply locally" for remote worktrees.
- **Approvals by policy:** `Policy::is_container` hosts can default to a broader permission mode for disposable sandboxes; never on Local, SSH or nodes. Destructive-command rules (voice `safety.rs`) apply everywhere.

## 7. Phases

Each phase ends with something a user can do, a test, and the size budget checked by `scripts/size_gate.sh`.

| Phase | Contents | Exit criteria |
| --- | --- | --- |
| **E0. Groundwork** (2–3 days) | `hosts/` module + `Provider` trait + `Endpoint`; `Host::{Docker, Node, Sandbox}` variants (serde-compatible); `cloud-keys.json` store; generic `probe` in the chip; Settings → Machines page listing what `discover()` finds | Existing WSL/SSH/Cua runs unchanged (`examples/smoke.rs`); old `host.<task>` values still load; unit tests for each variant's `wrap` |
| **E1. Solid SSH + Docker** (1 week) | Bundle push + patch sync (4.4 step 1); remote worktrees and diffs (4.4 steps 2–3); generic port forward (4.7); PTY panes (4.8); Docker host; "Install agent on host"; Add-a-machine for SSH/Docker | Unpushed commit + dirty file visible to Claude Code on an SSH host and in Docker; two agents on one remote repo in separate worktrees; terminal mode works over SSH; integration test against a local sshd container in CI |
| **E2. Headless node** (1.5–2 weeks) | `dove node` subcommands in the desktop binary (no GPUI init); copy-self / same-version fetch to the host; `attach`/reattach with replay (4.5); `mcp` bridge (4.6); QR + code pairing through the edge with the Woozlit account (§5); LAN-direct fast path | Pair a Mac from a Linux/Windows laptop; start Claude Code on the Mac from the laptop; close the laptop lid, approve from the phone, reopen and see the full transcript; browser tool works from the Mac session; `dove node serve` runs on a headless Linux server with no display |
| **E3. BYOK cloud, first wave** (1.5 weeks) | **InstaBox first** (owner's key, verified end to end); local Cua Space lifecycle (create/sleep/delete through `cua`, on this PC or a paired machine) as the free spare-capacity option; then Daytona, Boat, E2B, Fly Sprites, Codespaces, Coder (built from docs, "unverified" until tested); keys UI; fleet view; preview URLs; remote desktop tab | InstaBox: paste key → create box → run an agent on an unpushed branch → sleep → wake → finish → delete, scripted in `crates/dove-harness/examples/providers.rs` (run by hand with the owner's key, never in CI). Same script passes for a local Cua Space |
| **E4. BYOK cloud, second wave + local isolation** (1 week) | Modal (sidecar), Vercel (CLI), Fly Machines, Runloop, Morph, Ona; WSL Containers, Apptainer (local + over SSH), bubblewrap, smolmachines | Same scripted check per provider; Apptainer over SSH to an HPC-style host |
| **E5. Long tail on demand** | Northflank, Blaxel, Kubernetes, DeepInfra, Cloudflare Sandbox, Railway | Added when a user asks; each is one file in `hosts/` |
| **E6. Race across hosts** (ROADMAP P2) | Same prompt → N (agent, host) pairs → side-by-side diff/tests/cost/time → apply winner | The 30-second "three agents, three machines" video is real |

E0–E2 is ROADMAP Phase 1 and the start of Phase 3; E3 completes P5; E6 is P2.

## 8. What else to take from Hermes Agent

Hermes is one agent; Keplorayx orchestrates many. These adapt cleanly to a GUI over other agents' CLIs, ranked by value for effort. Paths are in `hermes-agent@ef75bc7`.

| # | Hermes feature | Keplorayx version | Where | Size |
| --- | --- | --- | --- | --- |
| 1 | **Account usage windows** (`agent/account_usage.py`: Claude OAuth 5 h / weekly / Opus windows, Codex 5 h / weekly, OpenRouter credits) | Show each subscription's remaining quota in Usage and next to the model picker; pick a host/agent with quota left in races | `pages/usage`, `dove-harness/catalog.rs` | S |
| 2 | **Tool-output caps with spill to disk** (`tools/tool_result_storage.py`: 50 KB / 2000 lines / 2000-char lines, spillover file + preview) | Cap transcript tool results in the engine recorder; full text in `~/.dove/spill/<id>.txt`, opened on click | `dove-core`, `dove-store` | S |
| 3 | **Fleet roster + Webapp** (`multi-connection-desktop.md`, `gateway/webapp_listener.py`) | Machines page (§6); later, `dove-node` serves a read-only web view for any browser with one-use links | `apps/dove`, `dove-node` | M |
| 4 | **Cron, `/loop`, heartbeat, Goals** (`cron/`, `features/loops.md`, `goals.md`) | P8: schedules run on a node or sandbox, results delivered to phone/notch/chat apps; "keep going until done" re-sends on idle | `dove-core/automation.rs` | M |
| 5 | **Messaging gateway** (27 platforms, `delivery_ledger.py`) | Pro: reach threads from Telegram, ntfy, Slack, Discord, email through the relay; a delivery ledger so no reply is lost | edge Worker + `dove-node` | M |
| 6 | **Checkpoints + `/rollback`** (`tools/checkpoint_manager.py`, shadow git store) | Per-turn checkpoints in the worktree's shadow refs, rewind button per turn (P6) | `dove-git` | M |
| 7 | **Session FTS5 search** (`hermes_state_fts.py`, CJK bigram tokenizer) | Search every transcript across agents and hosts from the sidebar | `dove-store` (SQLite already bundled) | S |
| 8 | **Import from other agents** (Claude Code, Codex, Cursor: instructions, MCP servers, skills) | First-run wizard imports MCP servers and rules; P9 single rules file synced to CLAUDE.md/AGENTS.md/GEMINI.md | first run | S |
| 9 | **Approval policies** (`tools/approval*.py`: smart guardian on a cheap model, unattended modes deny by default, denial breaker after 3, glob blocks even under yolo) | Per-host and per-schedule policy; unattended runs (cron, phone-started) default to deny-dangerous; a cheap local model triages requests | `dove-core`, voice `safety.rs` | M |
| 10 | **Live subagents frame with Steer/Stop** (`desktop.md:312`) | Team children shown above the composer of the parent thread | `apps/dove/orchestra.rs` | S |
| 11 | **Git review "Last turn" scope** (`tools/working_diff.py`) | Diff panel toggle: uncommitted / branch / last turn | `changes.rs` | S |
| 12 | **Skills curator** (`agent/curator.py`: usage tracking, stale archive, cheap-model review, audit ledger) and **write-approval gate** | Teach + skills catalog (P10) get usage counts, staleness and a review queue | `dove-harness/skills.rs` | M |
| 13 | **Plugin catalog with disclosure + trust tiers** (911 YAML entries pinned by SHA, mandatory "Disclosure —") | Same format for Keplorayx's integrations/skills/teammate templates catalog | `integrations_catalog.rs` | M |
| 14 | **Bot Screen take-over/hand-back lease** (`tools/bot_desktop/`) | The remote Desktop tab (4.7) with the existing computer-use lease | `computer_host.rs` | S (after 4.7) |
| 15 | **Kanban across machines** (`plugins/kanban`) | Existing kanban page gains a host column and a dispatcher that assigns cards to idle hosts | `pages/kanban` | M |
| 16 | **Tool search / progressive disclosure** (`tools/tool_search.py`) | Expose `cua__*` (156 tools, ~32k tokens) and Composio behind `tool_search`/`tool_call` in the `dove` MCP server | `mcp_browser.rs` | S |
| 17 | **Supply-chain checks** (`osv_check.py` on MCP packages, `threat_patterns.py` on context files, self-repo guard) | OSV check before installing a catalog MCP server; warn on prompt-injection patterns in AGENTS.md/CLAUDE.md of a cloned repo | catalog, first run | S |
| 18 | **HUD + Quick Entry hotkey** | Already covered by the notch; add a global Quick Entry on macOS/Linux like Voice Mode's Windows hotkey | `hotkey.rs` | S |
| 19 | **Hermes as an agent** | Already a Zeron driver; add `hermes acp` as an ACP agent so its users get Keplorayx's UI on any host | `acp.rs` | S |

Not taken: 39 model-provider plugins (agent CLIs bring their own), memory provider plugins (each agent has its own; P9 handles shared context), RL batch runner, voice wake word (Voice Mode covers it).

**Corrections to ROADMAP §9** (from the code at `ef75bc7`):

- Hermes **does** integrate Cua Spaces now, through Cua's own verified plugin (`plugin-catalog/cua-spaces.yaml`, `terminal.backend: cua`), which also uses `cua sandbox exec -i` for streams. Keplorayx's edge is many agents in one native GUI, not Cua access.
- `file_sync.py` syncs only Hermes's own skills, credentials and cache, never the project. The agent works on the remote filesystem directly.
- Singularity moved out of the repo into `NousResearch/hermes-official-plugins`.
- 39 model-provider plugins, not 40.
- Hermes has no GPU support on terminal backends, no persistent remote PTY and no device pairing. Remote machines are full Hermes gateways it connects to over HTTP or SSH.

## 9. Risks and open questions

| Risk | Mitigation |
| --- | --- |
| Provider APIs change monthly | One file per provider; `probe` reports drift as "needs setup" instead of failing runs; scripted per-provider check in E3 |
| Session caps (Modal / Vercel 24 h, E2B Hobby 1 h) | Show `max_lifetime` in the chip; `dove-node` checkpoints the transcript; warn 10 min before |
| Cost surprises on the user's account | Default to the smallest size, auto-sleep 15 min, TTL; show running boxes in the fleet view and the notch |
| Keys leaking to agents | `strip_env`, keys only in the launcher process, never in transcripts; redact `*_API_KEY` patterns in tool output |
| `dove-node` size and trust | Separate on-demand download, pinned SHA-256, signed manifest (ROADMAP §7), open source |
| Relay cost at scale | LAN/Tailscale direct first; relay carries events, not repos |
| Windows OpenSSH has no ControlMaster | Keep one long-lived `ssh` per host via `dove-node attach` multiplexing instead |

Decisions (owner, 2026-10-29):

1. **Pairing requires the Woozlit account.** QR or code binds a machine to the account; no account-free LAN pairing.
2. **InstaBox is the only cloud key on hand**, so it is the first cloud provider built and the only one verified end to end with a real account. The others are built against their documented APIs and marked "unverified" in the UI until tested. **Local Cua Spaces are the answer for spare capacity on your own computer:** if a machine has disk and RAM to spare, Keplorayx creates and runs local Spaces there (on this PC, or on a paired machine with `on: host:<machine>`), at no cost.
3. **No separate `dove-node` download.** The desktop binary runs headless as `dove node …` (§4.5).

Sources: E2B [streaming](https://docs.e2b.dev/commands/streaming), [SSH](https://docs.e2b.dev/sandbox/ssh-access.md), [lifetime](https://docs.e2b.dev/faq/sandbox-lifetime.md) · Daytona [SSH](https://www.daytona.io/docs/en/ssh-access/), [PTY](https://www.daytona.io/docs/en/pty/) · Modal [sandboxes](https://modal.com/docs/guide/sandbox) · Vercel [CLI](https://vercel.com/docs/sandbox/cli-reference) · Cua [cloud guide](https://cua.ai/docs/cua-sdk/guides/cloud) · InstaBox [SSH](https://instabox.sh/docs/ssh.md) · Boat [API](https://docs.boat.dev/api/v1.md), [OpenAPI](https://docs.boat.dev/openapi/boat-v1.yaml) · Runloop [SSH](https://docs.runloop.ai/docs/devboxes/ssh.md) · Morph [docs](https://cloud.morph.so/docs/developers) · Fly [Machines exec](https://docs.machines.dev/machines/Machines_exec.md), [Sprites exec](https://docs.sprites.dev/api/v001-rc48/exec) · Ona [CLI](https://ona.com/docs/ona/integrations/cli.md) · Northflank [sandboxes](https://northflank.com/docs/v1/application/sandboxes) · Blaxel [processes](https://docs.blaxel.ai/Sandboxes/Processes.md) · Cloudflare [PTY](https://developers.cloudflare.com/changelog/post/2026-02-09-pty-terminal-support/index.md). Pricing and limits were read on 2026-10-29; re-check before quoting.
