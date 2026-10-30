# Machines: run agents anywhere

Keplorayx can run any agent on this computer, in WSL, over SSH, in a container, on another computer of yours, in a Cua Space, or in a cloud sandbox you bring a key for. The transcript, approvals, diff, browser and team tools, phone and notch work the same wherever the agent runs.

Pick where a new thread runs with the **machine chip** in the composer footer (it shows your computer's name). A thread keeps its machine; resume and fork reuse it. **Settings › Machines** lists every machine with a live status, holds your cloud keys, and manages cloud boxes.

## What happens when you start a thread on another machine

1. **The machine is woken** if it's a sleeping Space or cloud box.
2. **Your project is copied there**, including commits you haven't pushed and edits you haven't committed (untracked files too). Later runs send only what changed. Ignored files (build output, `node_modules`) are not copied; the agent installs dependencies there.
3. **Keplorayx installs itself there** (`~/.dove/bin/dove`, same version as yours) when the machine has the same OS and CPU, so the agent runs under a small session keeper. If your connection drops or your laptop sleeps, the agent keeps working and Keplorayx reconnects for up to 30 minutes, replaying everything you missed. Set `DOVE_NODE=0` to turn this off for a machine where nothing may be installed.
4. **Keplorayx's own tools** (browser, team, `show_ui`, integrations) reach the agent through a reverse SSH tunnel.
5. **After every turn, the agent's work comes back** into your local folder, so the diff, history and file views show it. Nothing you changed locally is ever overwritten: if both sides changed, Keplorayx leaves both alone and says so.

The agent itself must be installed on the machine. If it isn't, the thread stops and shows the install command (for example `npm install -g @anthropic-ai/claude-code`).

## This computer and machines you already have

| Machine | What you need | Notes |
| --- | --- | --- |
| **WSL** (Windows) | An installed distro | Uses your Windows folder directly (`/mnt/c/…`); nothing is copied |
| **SSH** | A `Host` entry in `~/.ssh/config` that logs in without a password (key or ssh-agent) | Any VPS, home server or cloud VM. Connections are shared (ControlMaster) except on Windows |
| **Docker** | A running container with `sh` and `git` | Listed from `docker ps` |
| **Paired computer** | Keplorayx on both computers, signed in to the same account | See below |
| **Cua Space** | The `cua` CLI (`curl -fsSL https://cua.ai/install.sh \| sh`) | Local Spaces are free and use your spare disk and RAM; also on another machine of yours (`on=host:<machine>`) or your own AWS/GCP/Modal account |
| **Bubblewrap** (Linux) | `bwrap` | A private home for the agent on this computer; your agent logins aren't shared into it |
| **Apptainer / Singularity** | `apptainer` or `singularity` | Instances from any image (default `node:22-bookworm`) |
| **WSL Containers** (Windows) | WSL 2.9.3 or later | |
| **smolmachines** | `smolvm` (macOS Apple Silicon, Linux with KVM) | microVMs |
| **Kubernetes** | `kubectl` with a working context | A box is a pod: `<context>/<namespace>/<pod>` |
| **GitHub Codespaces** | `gh`, signed in | New codespaces need the repository as `owner/repo[@branch]` |

## Connecting another computer (your Mac, a home PC)

Pairing needs your Keplorayx account on both computers and opens no ports: both sides connect out to the relay.

1. On the computer that will run agents: **Settings › Machines › Use this computer as a machine**, or on a server without a screen, `dove node pair`. It shows a QR code and a short code.
2. On your laptop: machine chip › **Add a machine…** › **Pair a device**, then scan or type the code.
3. Pick the paired computer in the machine chip and start a thread. Close your laptop whenever you like; approve from your phone.

The paired computer only runs agents for computers it has paired with. Revoke one at any time in **Settings › Machines**.

**Headless servers.** The same Keplorayx binary runs without a window as `dove node …`. On a minimal Linux server it needs a few system libraries: `sudo apt-get install -y libxkbcommon0 libxkbcommon-x11-0 libxcb-xkb1 libasound2`. Keplorayx tells you exactly which ones are missing.

## Cloud sandboxes (bring your own key)

Paste a key in **Settings › Machines › Cloud providers**, press **Test**, then **New box** (or pick an existing one). Keys are stored in `~/.dove/cloud-keys.json`, readable only by you, and never reach an agent's environment. Boxes are created at the provider's smallest size; check your provider's pricing.

Providers marked **Unverified** are built from the provider's own API documentation and SDK sources but haven't been tested end to end with a real account yet.

| Provider | Key | How agents reach it | Also needs | Limits worth knowing |
| --- | --- | --- | --- | --- |
| InstaBox | `ibk_…` API key | SSH with a signed certificate | `ssh-keygen` (OpenSSH) | Free plan: 1 machine, auto-pause after 30 idle minutes |
| Daytona | API key | SSH (token login) | | |
| Boat | `boat_…` API key | SSH with a registered key | `ssh-keygen` | New sandboxes auto-stop after 1 hour idle |
| Runloop | API key | SSH through a TLS proxy | `openssl` | |
| Morph Cloud | API key | SSH with the instance key | | |
| Fly Machines | Fly API token | SSH to an sshd app over IPv6 | `ssh-keygen`, IPv6 | Alpine image |
| Coder | `https://coder.example.com\|<token>` | `coder ssh --stdio` | `coder` CLI | |
| Ona (Gitpod) | Personal access token | SSH from `ona environment ssh-config` | `ona` CLI | |
| Railway | Account or workspace token | SSH with your registered key | | Can't create or delete services from Keplorayx |
| E2B | `e2b_…` API key | E2B's process API | | Hobby plan: 1-hour sessions |
| Fly Sprites | Sprites token | WebSocket exec | | |
| Vercel Sandbox | `TOKEN\|TEAM_ID\|PROJECT_ID` | Vercel's command API | | 45 minutes (Hobby) / 24 hours (Pro) |
| Northflank | `token` or `token\|project` | WebSocket exec | | |
| Blaxel | `BL_API_KEY\|BL_WORKSPACE` | Blaxel's process API | | |
| DeepInfra | API key | DeepInfra's exec API | | Runs stop after 30 minutes |
| Modal | `TOKEN_ID:TOKEN_SECRET` | Modal's Python SDK | `pip install modal` | 24-hour maximum |
| Cloudflare Sandbox | — | Not available yet | | Needs a Worker of its own |

## Terminal mode

Switching a remote thread to **Terminal** opens the agent's own TUI on that machine with a real terminal. Keplorayx syncs first, like a chat run.

## Troubleshooting

| You see | What to do |
| --- | --- |
| `… is not installed on <machine>` | Install the agent there with the command shown, then try again |
| `has changes this PC doesn't have yet, so local edits were not sent` | The agent's remote work hasn't come back yet, and you also changed files here. Commit or stash one side |
| `lacks system libraries Keplorayx needs` | Run the `apt-get` command shown on that machine |
| `is <OS> <CPU>; install Keplorayx there` | The machine differs from yours, so Keplorayx can't copy itself. Install Keplorayx there for sessions that survive disconnects; agents still run without it |
| A cloud card says **Needs setup** | The key is missing or was rejected; paste a new one and press Test |
