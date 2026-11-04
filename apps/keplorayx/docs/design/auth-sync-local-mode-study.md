# Identity, sign-in, sync and local-only mode: a study

Status: **read-only audit. Nothing is implemented yet.** Every change listed under "Recommendation" waits for approval.

Sources read:
- Zeron's zgx app: `C:\dev\zeron\apps\zgx`, read-only.
- The original Dove at aa48d96: `C:\Users\ravth\Downloads\dove`, read-only.

Paths below are relative to those roots. Each finding is tagged:
- **[V]** verified in code
- **[A]** an assumption
- **[Q]** an open question

## 1. The short answer

- **Keplorayx's account is Woozlit, on the Cosmos edge. It is kept as it is.** *Decision (2026-11-04): Woozlit/Cosmos is Keplorayx's current account and stays. The edge and old website are not renamed yet.*
  - **[V]** The `dove-mesh-host` sidecar owns the Woozlit session. It signs in through a loopback callback, the same flow as the Cosmos desktop, and exchanges a Firebase ID token for a personal workspace where the uid is both user and org.
  - **[V]** It talks to the Cosmos edge, `https://cosmos-edge.cosmos-edge.workers.dev`.
  - **[V]** On top of that account sit phone remote control, Cloud threads, paired devices ("Use this computer as a machine") and the post-sign-in sync offer ("bring" or "fresh").
- **Every other sign-in is a provider's own:**
  - the agent CLIs' logins (read-only)
  - MCP servers (OAuth 2.1 + PKCE)
  - Supermemory, through that OAuth
  - cloud sandbox hosts (API keys)
- **Zeron's account** is a WorkOS sign-in owned by a separate engine process, with orgs and "sync" that switches the engine's workspace from local to synced. It is used here only as a pattern reference. Keplorayx does not adopt its endpoints, and does not need its account, because Woozlit already fills that role.
- **Recommendation:**
  - Keep the Woozlit/Cosmos account, the sidecar protocol, the endpoints and the stored files **unchanged**. Port the desktop bridge (`remote.rs`) into `crates/dove-app` without changing behaviour. `dove-mesh-host` stays a sidecar built from `mesh/`.
  - Signing in stays **optional**. Local-only (signed out, or signed in with remote off) is the default and must keep working with no account.
  - Publishing projects and sessions stays gated, as it is today, on *signed in **and** remote control on*. The sync offer is shown only after sign-in, and is never pre-accepted.
  - Provider and harness sign-ins stay separate, per provider, and are never merged into the Keplorayx account.

## 2. Data-flow diagram

```
                        ┌────────────────────── this device ──────────────────────┐
                        │                                                         │
  agent CLIs' own  ───► │ agent_auth.rs  (reads ~/.claude*, ~/.codex, cursor)     │  read-only, no network
  login files           │ quota.rs ─────────────────────────────────────────────► │──► api.anthropic.com / chatgpt.com
                        │                                      (CLI's own token)  │    (usage only)
                        │ dove-store: SQLite (WAL) + JSONL transcripts            │  local only
                        │   settings, projects, tasks, usage, transcripts         │
                        │ $DOVE_HOME/mcp_auth.json  (MCP OAuth grants)            │  plain file; 0600 on Unix only
                        │ $DOVE_HOME/hosts/<provider>-<id>/  (sandbox keys)       │  write_private(); 0600 on Unix only
                        │                                                         │
                        │ mcp_oauth.rs ── PKCE via browser + 127.0.0.1 loopback ─►│──► user-chosen MCP server / its AS
                        │ supermemory.rs (memory, opt-in by signing in) ─────────►│──► Supermemory API + MCP
                        │ remote.rs ── dove-mesh-host sidecar (stdin/stdout JSON)►│──► Woozlit sign-in (Firebase) +
                        │   = the Keplorayx account; $DOVE_HOME/mesh/ files       │    cosmos-edge.cosmos-edge.workers.dev
                        │   publishes projects/sessions only if signed in AND     │    (registry room, chat2 rooms,
                        │   remote on; node mode / paired devices; Cloud threads  │     device relay)
                        │ harness sessions ── prompts, files, tool output ───────►│──► the model provider the user picked
                        │ updates.rs ── version checks ──────────────────────────►│──► npm / GitHub releases
                        └─────────────────────────────────────────────────────────┘

  Zeron (for contrast):  app ──local IPC──► engine (owns session.json, WorkOS sign-in,
                         refresh, org, sync)  ──► edge.zeron.sh / staging.zeron.sh
```

## 3. Findings table

| Capability | Zeron implementation | Dove implementation | Keplorayx recommendation | Evidence | Open questions |
|---|---|---|---|---|---|
| App account | **[V]** A WorkOS hosted sign-in in the system browser, owned by the engine. The app holds no session. States: `restoring / signed_out / waiting / signed_in`. | **[V]** **Woozlit**, owned by the `dove-mesh-host` sidecar: `sign_in`, `cancel_sign_in` and `sign_out` ops over JSON lines, with `signedIn` and `sign_in_url` events back. The sidecar exchanges a Firebase ID token for a personal workspace (uid = user = org). Firebase rotates the refresh token, and `TokenProvider` single-flights refreshes. | **Keep it unchanged** (user decision): same sidecar, same ops, same edge. The Zeron *shape* already matches: one owner process, and scripts see only `kx.account` = `{ state = signed_out \| signing_in \| signed_in, … }`. The bearer never reaches Luau. | D `apps/dove/src/remote.rs:1-8`, `:266-285`, `:391-431`; `mesh/crates/host/src/main.rs:1-19`; `mesh/crates/client/src/auth.rs:1-33`, `:255`, `:297-357`; Z `src/auth.rs:1-70`, `:668` | **[Q]** The edge and website will be renamed later. Keep endpoints in one place (`mesh/crates/client/src/auth.rs` consts) so that rename is a single change. |
| Session restore | **[V]** The engine keeps `session.json` in its data dir. `session_exists()` skips the intro when signed in. | **[V]** The sidecar saves the account to `$DOVE_HOME/mesh/…` with `write_private` and restores it at spawn. Per-provider grants load from `mcp_auth.json` at use. | Keep both. Onboarding never blocks on the account: a returning signed-in user skips the account card. | Z `src/auth.rs:433`, `src/main.rs:253`; D `mesh/crates/host/src/main.rs:277-300`, `:338`, `:710`; `crates/dove-harness/src/paired.rs:11-12`; `mcp_oauth.rs:44-90` | |
| Token refresh/expiry | **[V]** The engine refreshes. The app re-asks for a bearer near expiry and retries once on refusal. | **[V]** MCP: `token()` refreshes when stale, and on 401 `refresh()` keeps the old refresh token when it isn't rotated. Supermemory retries once after a refresh. | Port as is into `crates/dove-app`. Add an `auth_expired` state per provider. | Z `src/auth.rs:104`, `:689-700`; D `mcp_oauth.rs:96-130`, `supermemory.rs:199-220` | |
| Harness/agent sign-in | **[V]** Agents' "Cloud logins" are the engine's. `ZERON_ZGX_ENGINE_HOME` isolates them for testing. | **[V]** `agent_auth::detect` reads each CLI's own files (Claude `.claude.json` / `.credentials.json`, Codex, Cursor). Other CLIs are `Unknown`. No network. | Keep it. It is read-only and local. Show it in onboarding's "Your agents" step as provider sign-in, never as a Keplorayx account. | D `agent_auth.rs:1-60`; Z `src/engine/manager.rs:30-36` | **[Q]** On Windows, does Claude Code keep credentials only in `.credentials.json`? (macOS uses the Keychain, so it reports `SignedIn(None)` or `Unknown`.) |
| MCP / integration OAuth | **[V]** Plugin network access is guarded by a host allow-list (`plugins/network.rs`). Account calls go through the engine. | **[V]** RFC 9728 → 8414 → 7591 dynamic registration → PKCE S256 with a one-shot `127.0.0.1` redirect. One grant per server, shared to every harness through `--mcp-browser`. | Keep it unchanged (preserves behaviour). Move it into `dove-app`. | D `mcp_oauth.rs:1-13`, `:217-350`; Z `src/plugins/network.rs:1-13` | |
| Secret storage | **[V]** Earlier builds used the macOS Keychain (`sh.zeron.zgx`). Now the engine's `session.json`. | **[V]** Plain JSON files. `0o600` **only on Unix**; on Windows they get the default ACL of `%USERPROFILE%`. The macOS Keychain is read only for Claude's quota token. | Phase E: move Dove-owned secrets (`mcp_auth.json`, host keys) to the OS store — Windows Credential Manager/DPAPI, macOS Keychain — with a file fallback and a one-time migration. Ask before migrating real data. | D `mcp_oauth.rs:52-75`, `crates/dove-harness/src/hosts/mod.rs:280-296`, `crates/dove-core/src/quota.rs:4-10`; Z `src/auth.rs:69` | **[Q]** `keyring` crate, or `windows-sys` CredWrite + DPAPI directly? Credential Manager has a 2.5 KB blob limit, and MCP grants may need splitting. |
| Cloud sync | **[V]** Sync is the engine's workspace mode: local while signed out, and a fresh synced workspace once signed in (the engine restarts). Sign-out stops the synced engine and restarts it locally. | **[V]** Woozlit-account sync, through the sidecar. It runs only when signed in **and** `remote_enabled=1`: projects and sessions go to the registry room `reg1/{org}/{user}`, and one Loro doc per chat goes to its chat2 room. After sign-in a **sync offer** asks what to do with this computer's work: `sync_claim = bring` (publish existing threads) or `fresh` (start empty). It is recorded once with `sync_offer_seen`. `account_skip.<id>` keeps a thread local. Cloud threads (`cloud_session.rs`) exist only after a Woozlit session and only for threads started on Cloud. Supermemory is separate, third-party and opt-in. | **Keep it unchanged.** The onboarding Privacy step and Settings state plainly what "remote on" publishes and to whom, and the sync offer stays an explicit choice after sign-in. | Z `src/engine/manager.rs:42-47`; D `remote.rs:1-8`, `:22-24`, `:149-175`, `:292-364`, `:386-395`; `mesh/crates/host/src/main.rs:5-19`; `crates/dove-core/src/cloud_session.rs:1-6`; `supermemory.rs:1-15` | **[Q]** The Loro docs merge automatically (CRDT). Is anything above them (registry rows, deletes, `account_skip`) conflict-checked? To be read in `mesh/crates/sync` before Phase E. |
| Local persistence | **[V]** The engine's data dirs (`.zeron`, `.zeron-staging`). Dev builds refuse production's. | **[V]** `dove-store`: SQLite in WAL mode + append-only JSONL per task under `$DOVE_HOME` (default `~/.dove`). | Keep the format identical (handoff rule). Dev builds use `%LOCALAPPDATA%\keplorayx-zgx-dev`. | D `crates/dove-store/src/lib.rs:1-5`, `:931-960`; `hosts/mod.rs:212-218`; Z `manager.rs:52-60` | |
| Third-party traffic | **[V]** The plugin guard denies private, metadata and LAN addresses unless the manifest names them. | **[V]** Harnesses send prompts/files to the user's chosen model. Plus quota checks, update checks (npm/GitHub), voice models (Hugging Face), sandbox hosts, MCP servers. | The Privacy step and Settings list each outbound class, with on/off where one exists. | D `quota.rs`, `updates.rs:1-5`, `crates/dove-voice/src/models.rs`, `crates/dove-harness/src/hosts/*` | **[Q]** Should update/quota checks be switchable off in strict local-only? |
| Secrets in memory/logs | **[V]** The bearer is never exposed to scripts. | **[V]** Memory saves redact credential-like values and skip `.env*`, `*.pem`, `id_rsa*`… | Keep the redaction. Add tests that logs, crash output and `--inspect` carry no tokens. | D `crates/dove-core/src/memory.rs:184`, `:475-525`; Z `auth.rs:18-21` | **[Q]** Audit the `log::`/`eprintln!` sites in `mcp_oauth.rs` and `hosts/*` before shipping. |
| Sign-out / disconnect | **[V]** `account.sign_out` → the engine's `SignOut`. A synced engine restarts locally. | **[V]** Account: `remote.sign_out` sends the sidecar's `sign_out` and clears `sync_offer_seen` and `sync_claim`, so the next sign-in asks again. Local threads stay. Per provider: `mcp_oauth::sign_out(id)` removes the grant. Remote can be switched off; signed out or off, "nothing leaves the machine". | Keep separate verbs: *Sign out of Keplorayx* (the Woozlit account), *Disconnect <provider>*, *Turn off remote*, *Clear local data* (the whole `DOVE_HOME`, with confirmation). Account deletion is the website's job until the edge is renamed. | D `remote.rs:4-5`, `:175`, `:391-396`; `mcp_oauth.rs:85` | **[Q]** Does sign-out delete what is already published to the rooms, or only stop publishing? **[Q]** Does disconnecting revoke at the server (RFC 7009)? Dove today only forgets. |

## 4. Local-only mode, defined

**Local-only** (the default) means:
1. No Keplorayx (Woozlit) account is needed. Signing in is optional and offered, never required.
2. All app data stays under `DOVE_HOME`:
   - projects, threads and transcripts
   - settings and team/persona data
   - grants and keys
3. Keplorayx sends **no app data to the Cosmos edge** unless the user is signed in **and** remote control is on, or they start a Cloud thread. Being signed in with remote off publishes nothing; that is the code's own rule (`mesh/crates/host/src/main.rs:18-19`), and a test must prove it.
4. Supermemory is disconnected unless the user connects it.

It does **not** stop traffic the user chose:
- the model provider a harness talks to (prompts, files, tool output)
- MCP servers they connected
- pages opened in the browser
- sandbox hosts

The Privacy step says so in plain words.

**Proof, not labels:** add a test harness that runs the engine with a fake agent under a deny-all network policy. Then assert that no socket opens other than loopback and the fake provider. **[A]** This is feasible on Windows with a per-process WFP rule, or more simply by routing reqwest through a recording proxy env var. To be decided when the tests are written.

## 5. UX states (for the onboarding "Privacy" step and Settings)

| State | Meaning | Surface |
|---|---|---|
| Local-only (default) | No account. Nothing of Keplorayx's leaves the device. | A quiet line in Settings › Privacy; no badge. |
| Provider signed out / signed in | Per harness, MCP server or Supermemory. | The agent's orbiting body in onboarding, and a row in Settings. |
| Provider auth expired | Refresh failed. | The warm attention colour on that row only. One-click "Sign in again". |
| Account signed out / signing in / signed in | The Woozlit account. | An optional card in onboarding (skippable) and Settings › Account. "Signing in" waits on the browser and can be cancelled. |
| Sync offer | First sign-in on this computer. | One calm choice, "Bring this computer's threads" or "Start fresh". Neither is preselected. |
| Remote off / on | Phone control and publishing. | When on, the row names what is published (projects, sessions, transcripts) and to whom. |
| Account auth expired | The refresh failed. | The attention colour on the account row only. One click, "Sign in again". |
| Offline | No network. | Local work continues; account features show "offline" in place. |

Provider sign-in and the Keplorayx (Woozlit) account are **different words in different places**, never one "Sign in" button.

## 6. Smallest safe plan (needs approval for each item)

1. **Phase 1 (no behaviour change):** move `agent_auth.rs`, `mcp_oauth.rs` and `supermemory.rs` into `crates/dove-app` as they are. Move `remote.rs` there as well, minus GPUI: the sidecar spawn, the JSON-line ops and events, and the sync-offer settings stay exactly as they are. Keep building `dove-mesh-host` from `mesh/` and shipping it beside `keplorayx.exe`. Unit-test the existing PKCE, refresh and redaction paths.
2. **Phase C:** add a `kx.account` store (state, user and remote on/off, from the sidecar's events; actions `account.sign_in`, `account.cancel`, `account.sign_out`, `remote.set`, `sync.bring`, `sync.fresh`) and a read-only `privacy` store listing outbound classes, for onboarding and Settings.
3. **Phase E:**
   - OS credential storage with migration from the JSON files, behind approval
   - optional RFC 7009 revoke on disconnect
   - an "update/quota checks" switch
4. **Tests to add:**
   - a local-only network-denial test
   - provider-auth regressions (PKCE round trip against a local fake AS; refresh-on-401)
   - remote stays off unless `remote_enabled=1`
   - a lifecycle test (clear local data removes grants)
   - a no-secrets-in-logs test

   Secrets never go in fixtures.
   Also add an account regression test: a fake sidecar answering the JSON-line protocol covers sign-in, cancel, sign-out (clears the sync offer), and "signed in + remote off publishes nothing".
5. **Not without explicit approval:** renaming or replacing the Woozlit/Cosmos edge, a new account backend, a change to what syncs, telemetry, or any new remote service.

## 7. Licensing

Zeron's zgx app is MIT (see `THIRD_PARTY_NOTICES.md`). Nothing from `auth.rs` or `cloud.rs` is planned for porting. Only the pattern of "one owner process, scripts see a state word" is adopted, and it is written fresh.
