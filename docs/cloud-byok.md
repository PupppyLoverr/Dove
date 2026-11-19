# Cloud mode with your own key

Keplorayx can run your threads on a computer you already own, reached over
your own `ssh`. There is no provider, no account and no relay of ours in that
path: the far side is a whole engine (`keplorayx-engine headless`) that you
install and control, and this app is one client of it.

That choice is the whole of the feature's trust story, so this page says it
plainly: what a machine of yours can read, what a compromised one could
reach, what the engine's bearer grants, and what you are accepting when you
add one.

## What the app does when you add a machine

1. `ssh-keyscan` reads the machine's host key and shows you its SHA256
   fingerprint. **Nothing is sent until you accept it.** Accepting writes that
   key into this app's own file, `machines/<id>.known_hosts`, in the app's data
   folder; connecting later uses it and nothing else.
2. Connecting runs `ssh -N -L 127.0.0.1:<local>:127.0.0.1:<engine port>`.
   Everything between this computer and the machine — commands, the file
   channel, the IPC stream and its bearer — rides that one encrypted
   connection.
3. If the engine there isn't running, it is started over that channel
   (`keplorayx-engine headless`, detached, its own data directory and port) and
   its log is its own `<data>/logs/headless.log`.
4. The engine's bearer is read over the same channel (`cat <data>/ipc-token`).
   The engine mints a new one on every start; this app keeps it **in memory
   only** and never writes it to this computer.
5. The app's ordinary IPC client dials the loopback port with that bearer, and
   from then on threads, streaming, approvals, files and diffs are the ones it
   already speaks — against *that* engine and *that* data directory.

Neither trust anchor is ours to mint: the host key is the machine's, the
bearer is its engine's.

## What the remote machine can read

- **Its own data directory.** Its threads and transcripts live there, plus
  every key you chose to put there (a model provider's, a git token) and any
  file the agents it runs have touched.
- **The project it works in.** Whatever is checked out on it, and — through
  its own `git`/`gh` login — every repository that login can reach.
- **Anything else you left on it**, exactly as any login of yours could.

## What a compromised machine could reach

- **The model keys in its own environment.** An agent with a shell can read
  `ANTHROPIC_API_KEY`; there is no way around that on a machine that runs
  turns. The mitigations are yours: per-machine keys, spend limits, and
  subscription logins over API keys where you can.
- **Every repository its git token can push to.**
- **Every thread in its data directory** — all of them, not just the one you
  are watching.

## What it cannot reach

- **This computer.** Not its key store, not its files, not its other
  machines' threads. The connection is one way: this app dials the machine,
  the machine does not dial back.
- **Any capability of ours.** There is no account to escalate into, no service
  of ours holding your keys, and nothing syncs unless you turn sync on.

A compromised *desktop* with a machine configured can drive that machine's
ssh, so the SSH credential is the one worth hardening: your agent, one key per
machine, no reuse.

## What the bearer grants

The engine's IPC is built for a client it trusts — the app on the same machine.
Over a forward, this app is still "the app": the bearer can do everything the
local one can, and the engine has no per-call authority beyond the bearer
itself. What keeps it from being a credential on the wire is that it is only
ever spoken inside the ssh channel; the engine binds `127.0.0.1` on the
machine and nothing else. Never publish that port.

## What you are accepting when you add a machine

- That machine holds your work while it does it, and everything above is
  reachable from it.
- The host key is the only thing proving the machine is the machine. If it
  changes the app refuses loudly and sends nothing; a rebuild is a reason to
  remove and re-add, not to accept a new key quietly.
- Removing a machine here deletes a record and a pinned key from this
  computer. It stops nothing on that machine: its engine keeps running, its
  threads keep waiting, and your ssh access is untouched.
- With no machine added, none of this exists: the app is local-only, and
  nothing about it suggests otherwise.

## Keys, and where they live

| Credential | Kept | Never |
|---|---|---|
| SSH private key | your `~/.ssh`, or the ssh agent (a passphrase belongs there: this app never prompts) | in this app's files |
| A model provider key the machine's engine needs | on that machine, `0600`, in its own engine environment | in this app's files, a thread, a log or a report |
| The engine's bearer | memory, for the life of the link | on disk, here or there (the engine writes its own) |
| The host key | `machines/<id>.known_hosts`, in the app's data folder | — (it is public) |

`machines.json` — the address, the port, the path to your key (or none), the
engine's port and data directory, the pinned host key — is not a secret. That
is deliberate: everything that *is* secret lives where the thing it authorizes
happens.
