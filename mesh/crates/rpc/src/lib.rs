//! zeron-rpc — the typed control plane (UiRpc / ControlRpc) over WebSocket + in-memory
//! transports, plus the device-room relay transport ({s,k,to,from} frames — [`device_room`]).
//!
//! Framing: ndjson envelopes, one JSON object per WebSocket text message (or per line on
//! byte transports), matching the shape of zeron's Effect RPC without the Effect runtime:
//!
//! - client → server: `{id, method, params}` to invoke, `{id, cancel: true}` to stop a stream;
//! - server → client: `{id, ok}` / `{id, err}` for unary calls,
//!   `{id, item}`* then `{id, done: true}` (or `{id, err}`) for streams.
//!
//! The server dispatches into an [`RpcService`]; the [`RpcClient`] offers `call` and
//! `subscribe`. Both ends run over any pair of string channels, so the in-memory transport
//! ([`memory_client`]) exercises the exact same code path as the WebSocket one.

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

mod client;
pub mod device_room;
mod server;

pub use client::{RpcClient, RpcSubscription, connect_ws};
pub use device_room::{
    ConnWrapper, DeviceFrameHeader, DeviceLink, HostRelay, HostRelayConfig, LinkCache,
    LinkCacheConfig, NudgeHandler, PeerLiveness, PeerLivenessProbe, StaticToken, TokenError,
    TokenSource, decode_device_frame, device_room_ws_url, encode_device_frame,
};
pub use server::{serve_connection, serve_ws_listener};

/// RPC method names — single source of truth for both ends.
/// Full surface: docs/research/feature-inventory.md §2.
pub mod methods {
    pub const WATCH_PREVIEWS: &str = "WatchPreviews";
    pub const LIST_HARNESSES: &str = "ListHarnesses";
    pub const CANCEL_INSTALL: &str = "CancelInstall";
    pub const INSTALL_HARNESS: &str = "InstallHarness";
    /// Flip a harness's enablement on the target device (Settings → Providers);
    /// replies with the device's fresh `ListHarnesses` catalog.
    pub const GET_TITLE_SETTINGS: &str = "GetTitleSettings";
    pub const SET_TITLE_SETTINGS: &str = "SetTitleSettings";
    pub const SET_HARNESS_ENABLED: &str = "SetHarnessEnabled";
    pub const LIST_MODELS: &str = "ListModels";
    pub const LIST_SKILLS: &str = "ListSkills";
    pub const LIST_COMMANDS: &str = "ListCommands";
    /// The harness's resumable session index (`/resume` picker):
    /// `{harness}` → `HarnessSession[]`.
    pub const LIST_SESSIONS: &str = "ListSessions";
    /// Attach a chat to a harness-native session: imports whatever transcript
    /// the store exposes into the doc, then stamps the chat's
    /// `harnessSessionId`/`harnessSessionCwd` so the next send resumes it.
    /// `{chatId, harness, sessionId, cwd}` → `{imported}`.
    pub const RESUME_SESSION: &str = "ResumeSession";
    /// A chat transcript's tail as flat rows — the `@chat` mention payload
    /// the composer folds into the outgoing prompt.
    /// `{chatId, maxRows?}` → `SessionTranscriptRow[]`.
    pub const CHAT_CONTEXT: &str = "ChatContext";
    pub const QUEUE_COMMAND: &str = "QueueCommand";
    pub const TAKE_PROJECT_ACTION_SETUP: &str = "TakeProjectActionSetup";
    /// Peer-to-peer delivery fallback: the SENDER's engine forwards a queued
    /// command entry (client-minted id and all) straight over the device-room
    /// link when its chat2 rows can't reach the edge but the host's peer link
    /// is alive. The host claims the id in its processed ledger before
    /// executing, so the doc row arriving later dedupes to a no-op —
    /// exactly-once by construction. Params `{chatId, entry}`.
    pub const RELAY_COMMAND: &str = "RelayCommand";
    /// User-driven delivery retry for a chat with unadopted queued sends:
    /// fresh chat2 socket, host nudge, drain pass, and a new delivery escort
    /// per pending command. Params `{chatId}`; IPC-only.
    pub const RETRY_DELIVERY: &str = "RetryDelivery";
    pub const WATCH_DOC_MESSAGES: &str = "WatchDocMessages";
    /// Explicit user navigation, `{chatId}`. Prioritizes this device's sync
    /// connection; automatic subscriptions and retries must not call it.
    pub const FOCUS_CHAT: &str = "FocusChat";
    /// Messages typed while the agent was busy, held on the chat doc so every
    /// device sees the same queue. `{ chatId }` → `{ items: QueuedMessage[] }`.
    pub const WATCH_QUEUE: &str = "WatchQueue";
    /// Append to the queue. `{ chatId, text, attachments?, holdForTurnEnd? }` → `{ id }`.
    pub const QUEUE_MESSAGE: &str = "QueueMessage";
    /// Retype a queued message; empty text deletes it.
    /// `{ chatId, id, text }` → `{ changed }`.
    pub const UPDATE_QUEUED_MESSAGE: &str = "UpdateQueuedMessage";
    /// Acquire a host-authoritative edit lease for one queued row.
    pub const BEGIN_QUEUED_MESSAGE_EDIT: &str = "BeginQueuedMessageEdit";
    /// Renew an acquired queue edit lease.
    pub const RENEW_QUEUED_MESSAGE_EDIT: &str = "RenewQueuedMessageEdit";
    /// Commit, cancel, discard, or explicitly release an acquired edit.
    pub const FINISH_QUEUED_MESSAGE_EDIT: &str = "FinishQueuedMessageEdit";
    /// Reorder. `{ chatId, id, toIndex }` → `{ changed }`.
    pub const MOVE_QUEUED_MESSAGE: &str = "MoveQueuedMessage";
    pub const REMOVE_QUEUED_MESSAGE: &str = "RemoveQueuedMessage";
    /// Interrupt whatever is running and send this one. `{ chatId, id }` → `{ sent }`.
    pub const SEND_QUEUED_MESSAGE_NOW: &str = "SendQueuedMessageNow";
    /// Steer this row into the live turn without interrupting it.
    /// `{ chatId, id }` → `{ sent }`.
    pub const STEER_QUEUED_MESSAGE_NOW: &str = "SteerQueuedMessageNow";
    /// Nudge every open room client to verify liveness NOW (window focus,
    /// app foregrounded). No params; IPC-only. Each room ignores the hint
    /// unless it has been broadcast-quiet ≥30s, so this is cheap to spam.
    pub const PROBE_SYNC: &str = "ProbeSync";
    /// Live sync introspection (`zeron sync` / debug surfaces): per-room
    /// connection state, last pushed-frame/ack ages, rejoin/probe/resync
    /// counters for the workspace room and every open chat doc. No params;
    /// IPC-only.
    pub const SYNC_STATUS: &str = "SyncStatus";
    /// Pushed edge-connectivity posture (`zeron_proto::Connectivity`):
    /// current value first, then every change — the connection pill /
    /// composer-honesty / queued-badge feed. No params; IPC-only.
    pub const WATCH_CONNECTIVITY: &str = "WatchConnectivity";
    /// In-flight queued-attachment transfers (`zeron_proto::TransferProgress`
    /// list): current set first, then a fresh snapshot per landed chunk —
    /// the sending thumbnail's percent-ring feed. No params; IPC-only.
    pub const WATCH_TRANSFERS: &str = "WatchTransfers";
    pub const WATCH_CHATS: &str = "WatchChats";
    pub const WATCH_SIDEBAR_PREFERENCES: &str = "WatchSidebarPreferences";
    pub const WATCH_DEVICES: &str = "WatchDevices";
    pub const WATCH_SESSIONS: &str = "WatchSessions";
    /// Spaces registry (device+folder pairs) from the workspace doc.
    pub const WATCH_SPACES: &str = "WatchSpaces";
    /// Entity mutations against the workspace doc (feature-inventory §2 DataRpc).
    /// Params are tagged `{op: createChat|createSpace|renameSpace|deleteSpace|
    /// renameChat|setChatArchived|deleteChat|renameDevice|markChatSeen, …}`.
    pub const MUTATE: &str = "Mutate";
    /// This engine's identity → `{deviceId}` (IPC-only; never relay-forwarded —
    /// the answer is about whichever engine you are directly connected to).
    pub const LOCAL_DEVICE: &str = "LocalDevice";
    // Phone pairing (WAN remote control). PhonePairingOffer/ListPairedPhones/
    // RevokePhone run on the desktop's own IPC/UI surface (IPC-only). PairPhone
    // and AuthorizePhone are answered by the relay gate on the device-room
    // conn itself — they never reach EngineRpc.
    /// `{rotate?}` → `{deviceId, code, qr, expiresAt}` — the current or fresh
    /// pairing offer for Settings → Devices "Pair phone".
    pub const PHONE_PAIRING_OFFER: &str = "PhonePairingOffer";
    /// `{code, label?}` → `{deviceToken, grantId, deviceId}` — redeem the
    /// on-screen code on THIS relay conn (single-use, short-lived).
    pub const PAIR_PHONE: &str = "PairPhone";
    /// `{deviceToken}` → `{ok}` — a stored grant re-authorizing a fresh conn.
    pub const AUTHORIZE_PHONE: &str = "AuthorizePhone";
    /// `{}` → `{phones: [{id, label, createdAt, lastUsedAt}]}` (no tokens).
    pub const LIST_PAIRED_PHONES: &str = "ListPairedPhones";
    /// `{id}` → `{ok}` — revoke a phone: kills its live conns and deletes the
    /// stored token so it can never re-authorize.
    pub const REVOKE_PHONE: &str = "RevokePhone";
    /// Renew a desktop viewport's host-local embedded-browser lease.
    pub const REGISTER_BROWSER: &str = "RegisterBrowser";
    // Cosmos Computer (device-local desktop perception/control). All are
    // IPC-only and NEVER relay-forwarded: they report or act on whichever
    // engine you are directly connected to — forwarding a desktop action to
    // another machine's physical UI must be an explicit decision, not a
    // relay side-effect.
    /// Component/settings/capability snapshot → `zeron_computer::ComputerStatus`.
    pub const COMPUTER_STATUS: &str = "ComputerStatus";
    /// Persist `{enabled, semantic, screenshots, control, telemetry, …}`.
    /// Returns the resulting `ComputerStatus`.
    pub const COMPUTER_SET_SETTINGS: &str = "ComputerSetSettings";
    /// Download + verify + install the pinned driver component (streams
    /// `InstallProgress` items).
    pub const COMPUTER_INSTALL: &str = "ComputerInstall";
    /// Remove the installed component.
    pub const COMPUTER_UNINSTALL: &str = "ComputerUninstall";
    /// Live probe — driver ping + health report (OS permission grants).
    pub const COMPUTER_PROBE: &str = "ComputerProbe";
    /// One observe call → `zeron_computer::Observation` (screenshot rides as
    /// a staged uploads-jail path, readable via ReadAttachmentChunk).
    pub const COMPUTER_OBSERVE: &str = "ComputerObserve";
    /// Subscribe to component status changes (install progress, settings,
    /// driver liveness) — stream of `ComputerStatus` snapshots.
    pub const WATCH_COMPUTER: &str = "WatchComputer";
    // Cosmos Tools — the device-local merged MCP gateway (Composio Connect +
    // user-configured MCP servers). Unlike computer, ALL of these are
    // relay-forwardable: tool configuration is per-device account state, and
    // Settings → Tools targets the device hosting the agent via
    // `targetDeviceId`. Replies carry masked config only — API keys and
    // custom-server headers never cross the wire.
    /// Provider snapshot → `zeron_tools::ToolsSnapshot` (masked configs,
    /// per-provider status/tool counts, merged tool total).
    pub const LIST_TOOLS: &str = "ListTools";
    /// Add or update one provider (`ToolProvider`; the id is stable across
    /// edits). Returns the fresh `ToolsSnapshot`.
    pub const UPSERT_TOOL_PROVIDER: &str = "UpsertToolProvider";
    /// Remove one provider by id. Returns the fresh `ToolsSnapshot`.
    pub const REMOVE_TOOL_PROVIDER: &str = "RemoveToolProvider";
    /// `{id, enabled}` toggle. Returns the fresh `ToolsSnapshot`.
    pub const SET_TOOL_PROVIDER_ENABLED: &str = "SetToolProviderEnabled";
    /// `{apiKey}` — validate the Composio consumer key against Connect and
    /// store it. Returns the provider's `ProviderStatus` (key masked).
    pub const CONNECT_COMPOSIO: &str = "ConnectComposio";
    /// Remove the Composio provider entirely. Returns the fresh snapshot.
    pub const DISCONNECT_COMPOSIO: &str = "DisconnectComposio";
    /// Per-toolkit connection state → `Vec<zeron_tools::ComposioApp>` (the
    /// curated grid plus every remembered toolkit).
    pub const LIST_COMPOSIO_APPS: &str = "ListComposioApps";
    /// `{query}` — full-catalog search via `COMPOSIO_SEARCH_TOOLS` →
    /// `Vec<zeron_tools::ComposioSearchHit>`.
    pub const SEARCH_COMPOSIO: &str = "SearchComposio";
    /// `{toolkit}` — initiate one app's OAuth flow → `AppConnectStart`
    /// (carries the hosted auth URL the settings page opens).
    pub const START_APP_CONNECT: &str = "StartAppConnect";
    /// `{toolkit}` — poll one app's connection state → `ComposioApp`.
    pub const POLL_APP_CONNECT: &str = "PollAppConnect";
    /// Test a provider config WITHOUT persisting it → `ProbeResult`
    /// (tool count + sample names, or the connect error).
    pub const PROBE_MCP_SERVER: &str = "ProbeMcpServer";
    /// Provider status changes — stream of `ToolsSnapshot`s.
    pub const WATCH_TOOLS: &str = "WatchTools";
    /// This engine runtime's fixed device and workspace identity.
    pub const ENGINE_INFO: &str = "EngineInfo";
    /// Readiness barrier for the engine runtime. The call completes once stores
    /// and journals are assembled, or fails with the assembly error.
    pub const ENGINE_READY: &str = "EngineReady";
    /// Ask a headless IPC owner to drain its runtime and exit successfully.
    /// Headed IPC owners do not implement this method: closing another app's
    /// engine behind its windows would leave that process unusable.
    pub const STOP_ENGINE: &str = "StopEngine";
    pub const AUTH_STATUS: &str = "AuthStatus";
    // AuthRpc mutations (feature-inventory §2 AuthRpc; IPC-only).
    pub const SIGN_IN: &str = "SignIn";
    pub const SIGN_IN_HEADLESS: &str = "SignInHeadless";
    pub const COMPLETE_SIGN_IN: &str = "CompleteSignIn";
    pub const SIGN_OUT: &str = "SignOut";
    pub const LIST_ORGS: &str = "ListOrgs";
    pub const CREATE_ORG: &str = "CreateOrg";
    pub const SELECT_ORG: &str = "SelectOrg";
    /// One-time local→synced profile import: what's importable (unary).
    pub const LOCAL_IMPORT_STATUS: &str = "LocalImportStatus";
    /// One-time local→synced profile import: run it (stream of progress items).
    pub const IMPORT_LOCAL_WORKSPACE: &str = "ImportLocalWorkspace";
    /// `{}` → `[{chatId, title, updatedAt, archived}]` from this device's
    /// Local profile.
    pub const LIST_LOCAL_CHATS: &str = "ListLocalChats";
    /// `{localChatId, seatId}` → `{chatId}`; explicitly copies a Local chat
    /// to a new Cloud chat without mutating the source.
    pub const COPY_LOCAL_CHAT_TO_CLOUD: &str = "CopyLocalChatToCloud";
    // Repos / worktrees / folders (ControlRpc, relay-forwardable).
    pub const LIST_REPOS: &str = "ListRepos";
    pub const ADD_REPO: &str = "AddRepo";
    pub const CLONE_REPO: &str = "CloneRepo";
    pub const CREATE_REPO: &str = "CreateRepo";
    pub const LIST_BRANCHES: &str = "ListBranches";
    pub const LIST_REFS: &str = "ListRefs";
    pub const LIST_GIT_HISTORY: &str = "ListGitHistory";
    /// Fuzzy commit-subject / SHA search over the complete public history.
    pub const SEARCH_GIT_HISTORY: &str = "SearchGitHistory";
    /// Resolve hosted profile images for a page of Git commit authors.
    pub const RESOLVE_GIT_AVATARS: &str = "ResolveGitAvatars";
    /// Update remote-tracking refs without changing HEAD, the index, or files.
    pub const FETCH_ALL: &str = "FetchAll";
    pub const SWITCH_REF: &str = "SwitchRef";
    pub const LIST_FOLDERS: &str = "ListFolders";
    /// The device's browse roots: home plus mounted drives/volumes.
    pub const LIST_DRIVES: &str = "ListDrives";
    /// Fuzzy relative-path search rooted in a known chat or space checkout.
    pub const SEARCH_FILES: &str = "SearchFiles";
    // Device-local workspace filesystem operations. All are relay-forwardable;
    // WatchWorkspaceFiles is the only streaming method in this group.
    pub const LIST_WORKSPACE_DIRECTORY: &str = "ListWorkspaceDirectory";
    pub const SEARCH_WORKSPACE_FILES: &str = "SearchWorkspaceFiles";
    pub const READ_WORKSPACE_IMAGE: &str = "ReadWorkspaceImage";
    pub const READ_WORKSPACE_FILE: &str = "ReadWorkspaceFile";
    pub const WRITE_WORKSPACE_FILE: &str = "WriteWorkspaceFile";
    pub const WATCH_WORKSPACE_FILES: &str = "WatchWorkspaceFiles";
    pub const CREATE_WORKTREE: &str = "CreateWorktree";
    pub const DELETE_WORKTREE: &str = "DeleteWorktree";
    // Project Actions are private state on the device that owns the project.
    pub const LIST_PROJECT_ACTIONS: &str = "ListProjectActions";
    pub const UPSERT_PROJECT_ACTION: &str = "UpsertProjectAction";
    pub const DELETE_PROJECT_ACTION: &str = "DeleteProjectAction";
    pub const RUN_PROJECT_ACTION: &str = "RunProjectAction";
    // Terminals (ControlRpc, relay-forwardable; SubscribeTerminal streams).
    pub const OPEN_TERMINAL: &str = "OpenTerminal";
    pub const SUBSCRIBE_TERMINAL: &str = "SubscribeTerminal";
    pub const WRITE_TERMINAL: &str = "WriteTerminal";
    pub const RESIZE_TERMINAL: &str = "ResizeTerminal";
    pub const CLOSE_TERMINAL: &str = "CloseTerminal";
    /// Checkout-diff stream for the target device's chats (DataRpc,
    /// relay-forwardable — diffs are produced where the checkout lives).
    pub const WATCH_CHECKOUT_DIFFS: &str = "WatchCheckoutDiffs";
    pub const WATCH_WORKSPACE_GIT_STATUS: &str = "WatchWorkspaceGitStatus";
    /// Current pull request for one checkout, resolved on the checkout's host device.
    pub const WATCH_CHECKOUT_CHANGE_REQUEST: &str = "WatchCheckoutChangeRequest";
    pub const GET_CHECKOUT_DIFF: &str = "GetCheckoutDiff";
    /// Permanently restore one chat-owned checkout to its current HEAD and
    /// remove only its untracked, non-ignored paths.
    pub const DISCARD_WORKING_TREE: &str = "DiscardWorkingTree";
    pub const GET_CHECKOUT_FILE_DIFF_TEXT: &str = "GetCheckoutFileDiffText";
    // Agent accounts (ControlRpc, relay-forwardable — CLI logins are per-device).
    pub const LIST_AGENT_ACCOUNTS: &str = "ListAgentAccounts";
    pub const ACTIVATE_AGENT_ACCOUNT: &str = "ActivateAgentAccount";
    pub const FORGET_AGENT_ACCOUNT: &str = "ForgetAgentAccount";
    pub const START_AGENT_LOGIN: &str = "StartAgentLogin";
    pub const COMPLETE_AGENT_LOGIN: &str = "CompleteAgentLogin";
    pub const POLL_AGENT_LOGIN: &str = "PollAgentLogin";
    pub const CANCEL_AGENT_LOGIN: &str = "CancelAgentLogin";
    // Uploads / attachments (ControlRpc, relay-forwardable — target the chat's host device).
    pub const UPLOAD_CHUNK: &str = "UploadChunk";
    pub const UPLOAD_COMMIT: &str = "UploadCommit";
    pub const READ_ATTACHMENT_CHUNK: &str = "ReadAttachmentChunk";
    /// Lazy full-tool-output fetch from the R2 sidecar by doc-resident ref
    /// (chat2-sync A3). Edge-direct from any device — never relay-forwarded.
    pub const FETCH_TOOL_BLOB: &str = "FetchToolBlob";
    // Updates (ControlRpc, relay-forwardable — a device reports/applies its own
    // binary's update). Stream: current UpdateStatus, then every change.
    pub const UPDATE_STATUS: &str = "UpdateStatus";
    /// Download + apply the newest release on the target device (symlink-managed
    /// installs; the service restart is scheduled after the reply flushes).
    pub const APPLY_UPDATE: &str = "ApplyUpdate";
    // Cloud / GitHub agent mode — device-local, NEVER relay-forwarded: seat
    // config, BYOK keys, and the run ledger live in the engine's own data
    // dir, and a seat's exec surface must run where the ssh agent / tokens
    // actually are. Replies always carry redacted views (`has_*` booleans);
    // secrets never cross the wire.
    /// The redacted config + probe cache + run ledger → `cloud.view()`.
    pub const CLOUD_GET_CONFIG: &str = "CloudGetConfig";
    /// `{seat}` upsert (secrets as `{secrets:{boxToken?,githubPat?,cuaToken?}}`)
    /// → `{ok}`.
    pub const CLOUD_SAVE_SEAT: &str = "CloudSaveSeat";
    /// `{seatId}` → `{ok}` (also drops stored seat tokens).
    pub const CLOUD_REMOVE_SEAT: &str = "CloudRemoveSeat";
    /// `{seatId}` → `SeatProbe` (and records it as the seat's last_probe).
    pub const CLOUD_PROBE_SEAT: &str = "CloudProbeSeat";
    /// `{seatId}` → `{pending}` — long-polls Railway signup completion.
    pub const CLOUD_RAILWAY_SIGNUP_WAIT: &str = "CloudRailwaySignupWait";
    /// `{provider, key, baseUrl?}` → `{ok, message}` — a models/list ping.
    pub const CLOUD_VERIFY_PROVIDER: &str = "CloudVerifyProvider";
    /// `{provider, baseUrl?, model?, useForCloud, key?}` → `{ok}` — the key,
    /// when present, lands in `cloud-keys.json` (0600), never `cloud.json`.
    pub const CLOUD_SAVE_PROVIDER: &str = "CloudSaveProvider";
    /// `{provider}` → `{ok}` (config row + stored key both drop).
    pub const CLOUD_REMOVE_PROVIDER: &str = "CloudRemoveProvider";
    /// `{seatId, dir?}` → `{entries: [RemoteEntry]}`.
    pub const CLOUD_LIST_DIR: &str = "CloudListDir";
    /// `{seatId, path}` → `{content, truncated}` (256 KiB cap).
    pub const CLOUD_READ_FILE: &str = "CloudReadFile";
    /// `{seatId}` → `{branch, entries, stat}`.
    pub const CLOUD_GIT_STATUS: &str = "CloudGitStatus";
    /// `{seatId, path?}` → `{diff}` (unified, bounded).
    pub const CLOUD_GIT_DIFF: &str = "CloudGitDiff";
    /// `{seatId, command}` → `{code, stdout, stderr}` — the Shell pane's
    /// non-interactive path (PTY attach rides the local terminal + `ssh`).
    pub const CLOUD_EXEC: &str = "CloudExec";
    /// `{seatId?, query?}` → `{repos: [RepoHit], orgs: [String]}` — the
    /// GitHub-agent repo picker; resolves the seat's PAT or the shared one.
    pub const CLOUD_GITHUB_REPOS: &str = "CloudGitHubRepos";
    /// `{repo, seatId?}` → `{branches: [String], defaultBranch, truncated}` —
    /// the cloud composer's branch picker for the picked repo.
    pub const CLOUD_GITHUB_BRANCHES: &str = "CloudGitHubBranches";
    /// `{pat}` → `{ok}` — verify + store the shared GitHub PAT (repo scope)
    /// that the picker and every PAT-less GitHub seat fall back to.
    pub const CLOUD_GITHUB_SAVE: &str = "CloudGitHubSave";
    /// `{}` → `{deviceCode, userCode, verificationUri, interval, expiresIn}` —
    /// starts the Cosmos GitHub App device flow.
    pub const CLOUD_GITHUB_DEVICE_START: &str = "CloudGitHubDeviceStart";
    /// `{deviceCode}` → `{status: pending|slow_down|done, login?}` — one poll;
    /// `done` stores the user token as the shared GitHub credential.
    pub const CLOUD_GITHUB_DEVICE_POLL: &str = "CloudGitHubDevicePoll";
    /// `{}` → `{ok}` — forget the shared GitHub credential.
    pub const CLOUD_GITHUB_DISCONNECT: &str = "CloudGitHubDisconnect";
    /// `{seatId}` → `[PrRow]` (needs a bound repo + PAT).
    pub const CLOUD_GITHUB_PRS: &str = "CloudGitHubPrs";
    /// `{seatId, number}` → `PrDetail` (files + checks included).
    pub const CLOUD_GITHUB_PR: &str = "CloudGitHubPr";
    /// `{seatId, number}` → `{ok}` — squash merge via the API.
    pub const CLOUD_GITHUB_MERGE: &str = "CloudGitHubMerge";
    /// `{seatId}` → `{pngBase64}` | unavailable — rides the stamped CUA
    /// endpoint's screenshot surface.
    pub const CLOUD_SCREENSHOT: &str = "CloudScreenshot";
    /// `{automation}` upsert → `{ok}`.
    pub const CLOUD_AUTOMATION_SAVE: &str = "CloudAutomationSave";
    /// `{id}` → `{ok}`.
    pub const CLOUD_AUTOMATION_REMOVE: &str = "CloudAutomationRemove";
    /// `{chatId, seatId?, op}` — seat/run control for the bot bar:
    /// `pause|resume|stop|approve|destroy` on runs; `sleep|wake` manage the
    /// Boat sandbox lifecycle; `markKeepPrompted` records the one-shot
    /// "Keep this computer" card so it never re-asks.
    pub const CLOUD_SEAT_CONTROL: &str = "CloudSeatControl";
    /// `{seatId}` → `{command}` — the exact local shell line a Terminal pane
    /// spawns for an interactive seat shell (`ssh -tt …`), or an error.
    pub const CLOUD_SSH_COMMAND: &str = "CloudSshCommand";
    /// `{seatId, workspace?, repo?}` → `{status}` — bring the seat's virtual
    /// desktop up (installs Xvfb + xdotool when missing) and wait for it.
    pub const CLOUD_DESKTOP_START: &str = "CloudDesktopStart";
    /// `{seatId, action: SeatAction}` → `{ok, output}` — the Computer pane's
    /// "Take control": one click/type/key/scroll/open on the seat desktop.
    pub const CLOUD_COMPUTER_ACT: &str = "CloudComputerAct";
    /// `{chatId, seatId}` → `{ok, binding}` — move a chat to another seat
    /// (a dead trial's thread continuing on Boat); its workspace backup
    /// restores there on the next run.
    pub const CLOUD_MOVE_CHAT: &str = "CloudMoveChat";
    /// `{chatId, cwd?, pushWip?}` → `{ok, wipBranch?}` — return a Cloud
    /// conversation to this signed-in device without copying runner files.
    pub const CLOUD_RETURN_CHAT: &str = "CloudReturnChat";
    /// `{chatId, seatId, pushWip?}` → `{ok, binding, wipBranch?}` — bind a
    /// Synced seatless chat to an existing Cloud computer seat.
    pub const CLOUD_SEND_CHAT: &str = "CloudSendChat";
    /// Runner-local workspace dirt check; forwarded to the host device.
    pub const RUNNER_WORKSPACE_STATUS: &str = "RunnerWorkspaceStatus";
    /// Commit and push runner-local work on a one-off WIP branch.
    pub const RUNNER_PUSH_WIP: &str = "RunnerPushWip";
    /// `{seatId, deviceId|null}` → `{ok, seat}` — attach or detach a headless
    /// Cosmos runner already registered in this profile.
    pub const CLOUD_SET_SEAT_RUNNER: &str = "CloudSetSeatRunner";
    pub const RUNNER_INFO: &str = "RunnerInfo";
    pub const RUNNER_SEAL_CREDENTIAL: &str = "RunnerSealCredential";
    pub const RUNNER_REVOKE_CREDENTIAL: &str = "RunnerRevokeCredential";
    pub const RUNNER_SET_LOGIN: &str = "RunnerSetLogin";
    pub const CLOUD_SEAL_FOR_RUNNER: &str = "CloudSealForRunner";
    /// `{id}` → `{ok, chatId?}` — fire an automation now, off-schedule.
    pub const CLOUD_AUTOMATION_RUN_NOW: &str = "CloudAutomationRunNow";
    /// `{chatId}` → `{ok, backup}` — snapshot a chat's seat workspace to this
    /// computer now (the Context pane's "Back up now").
    pub const CLOUD_BACKUP_CHAT: &str = "CloudBackupChat";
    /// `{prompt, seatId?, spaceId?, harness?, start}` → `{chatId}` — Kanban
    /// "+ New task": creates the task's chat; `start` queues the run now,
    /// otherwise it lands in Draft.
    pub const CLOUD_TASK_CREATE: &str = "CloudTaskCreate";
    /// `{chatId}` → `{ok}` — start a Draft task (its first run queues).
    pub const CLOUD_TASK_START: &str = "CloudTaskStart";
    /// `{chatId}` → `{ok}` — drop a Draft task and its empty chat.
    pub const CLOUD_TASK_DISCARD: &str = "CloudTaskDiscard";
}

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("unknown method: {0}")]
    UnknownMethod(String),
    #[error("bad params: {0}")]
    BadParams(String),
    #[error("{0}")]
    Failed(String),
    #[error("transport: {0}")]
    Transport(String),
    #[error("connection closed")]
    Closed,
}

/// A client-originated frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientFrame {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel: bool,
}

/// A server-originated frame. Exactly one of `ok` / `err` / `item` / `done` is meaningful.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerFrame {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
}

/// What a service returns for one invocation.
pub enum RpcReply {
    /// Unary response — sent as `{id, ok}`.
    Value(serde_json::Value),
    /// Stream — each item sent as `{id, item}`, then `{id, done: true}` when it ends.
    Stream(BoxStream<'static, serde_json::Value>),
}

impl RpcReply {
    /// Serialize a value into a unary reply.
    pub fn value<T: Serialize>(value: &T) -> Result<Self, RpcError> {
        serde_json::to_value(value)
            .map(RpcReply::Value)
            .map_err(|e| RpcError::Failed(format!("serialize response: {e}")))
    }
}

/// Server-side dispatch: one implementation serves every transport.
#[async_trait]
pub trait RpcService: Send + Sync + 'static {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError>;
}

/// Deserialize typed params out of the envelope's `params` value.
pub fn parse_params<T: serde::de::DeserializeOwned>(
    params: serde_json::Value,
) -> Result<T, RpcError> {
    serde_json::from_value(params).map_err(|e| RpcError::BadParams(e.to_string()))
}

/// Spawn an in-memory server for `service` and return a connected client.
/// Same envelopes, same dispatch loop as the WebSocket path — the in-process UI
/// transport (ARCHITECTURE §1 "zero serialization shortcuts").
pub fn memory_client(service: Arc<dyn RpcService>) -> RpcClient {
    let (client_out, server_in) = tokio::sync::mpsc::channel::<String>(256);
    let (server_out, client_in) = tokio::sync::mpsc::channel::<String>(256);
    tokio::spawn(serve_connection(service, server_out, server_in));
    RpcClient::new(client_out, client_in)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::sync::Mutex;

    struct TestService;

    struct CancelAwareService {
        dropped: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    }

    struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(dropped) = self.0.take() {
                let _ = dropped.send(());
            }
        }
    }

    #[async_trait]
    impl RpcService for CancelAwareService {
        async fn handle(
            &self,
            method: &str,
            _params: serde_json::Value,
        ) -> Result<RpcReply, RpcError> {
            if method != methods::WATCH_CHECKOUT_CHANGE_REQUEST && method != "Silent" {
                return Err(RpcError::UnknownMethod(method.into()));
            }
            let guard = DropSignal(self.dropped.lock().unwrap().take());
            let stream = futures::stream::unfold(guard, |guard| async move {
                let item = std::future::pending::<Option<(serde_json::Value, DropSignal)>>().await;
                drop(guard);
                item
            });
            Ok(RpcReply::Stream(stream.boxed()))
        }
    }

    #[async_trait]
    impl RpcService for TestService {
        async fn handle(
            &self,
            method: &str,
            params: serde_json::Value,
        ) -> Result<RpcReply, RpcError> {
            match method {
                "Echo" => Ok(RpcReply::Value(params)),
                "Count" => {
                    let n = params.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
                    Ok(RpcReply::Stream(
                        futures::stream::iter((0..n).map(|i| serde_json::json!(i))).boxed(),
                    ))
                }
                "Never" => Ok(RpcReply::Stream(futures::stream::pending().boxed())),
                "Boom" => Err(RpcError::Failed("boom".into())),
                other => Err(RpcError::UnknownMethod(other.into())),
            }
        }
    }

    #[tokio::test]
    async fn memory_call_stream_and_error() {
        let client = memory_client(Arc::new(TestService));

        let echoed = client
            .call("Echo", serde_json::json!({"x": 1}))
            .await
            .unwrap();
        assert_eq!(echoed, serde_json::json!({"x": 1}));

        let mut items = client
            .subscribe("Count", serde_json::json!({"n": 3}))
            .await
            .unwrap();
        let mut seen = Vec::new();
        while let Some(v) = items.recv().await {
            seen.push(v);
        }
        assert_eq!(
            seen,
            vec![
                serde_json::json!(0),
                serde_json::json!(1),
                serde_json::json!(2)
            ]
        );

        let err = client
            .call("Boom", serde_json::Value::Null)
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::Failed(m) if m == "boom"));
    }

    #[tokio::test]
    async fn checked_stream_acknowledges_support_and_preserves_unknown_method() {
        let client = memory_client(Arc::new(TestService));

        let mut items = client
            .subscribe_checked("Count", serde_json::json!({"n": 1}))
            .await
            .unwrap();
        assert_eq!(items.recv().await, Some(serde_json::json!(0)));
        assert_eq!(items.recv().await, None);

        let error = match client
            .subscribe_checked("FutureStream", serde_json::Value::Null)
            .await
        {
            Ok(_) => panic!("old service must reject unknown stream"),
            Err(error) => error,
        };
        assert!(matches!(error, RpcError::UnknownMethod(method) if method == "FutureStream"));
    }

    #[tokio::test]
    async fn dropping_checked_subscription_cancels_pending_server_stream() {
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let client = memory_client(Arc::new(CancelAwareService {
            dropped: Mutex::new(Some(dropped_tx)),
        }));
        let stream = client
            .subscribe_checked(
                methods::WATCH_CHECKOUT_CHANGE_REQUEST,
                serde_json::Value::Null,
            )
            .await
            .unwrap();

        drop(stream);

        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .expect("server stream cancelled")
            .expect("drop signal");
    }

    #[tokio::test]
    async fn scoped_subscription_returns_before_first_item_and_cancels_silence() {
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let service = Arc::new(CancelAwareService {
            dropped: Mutex::new(Some(dropped_tx)),
        });
        let client = memory_client(service.clone());
        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            client.subscribe_scoped("Silent", serde_json::Value::Null),
        )
        .await
        .unwrap()
        .unwrap();
        while service.dropped.lock().unwrap().is_some() {
            tokio::task::yield_now().await;
        }
        drop(stream);
        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn websocket_round_trip() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(serve_ws_listener(listener, Arc::new(TestService)));

        let client = connect_ws(&format!("ws://127.0.0.1:{port}")).await.unwrap();
        let echoed = client
            .call("Echo", serde_json::json!("hello"))
            .await
            .unwrap();
        assert_eq!(echoed, serde_json::json!("hello"));

        let mut items = client
            .subscribe("Count", serde_json::json!({"n": 2}))
            .await
            .unwrap();
        assert_eq!(items.recv().await, Some(serde_json::json!(0)));
        assert_eq!(items.recv().await, Some(serde_json::json!(1)));
        assert_eq!(items.recv().await, None);
    }

    #[tokio::test]
    async fn handshake_with_origin_header_is_rejected() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(serve_ws_listener(listener, Arc::new(TestService)));

        // A remote browser page opening ws://127.0.0.1:{port} always sends a
        // non-local Origin; the server must refuse it before serving any RPC.
        let mut req = format!("ws://127.0.0.1:{port}")
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("origin", "https://evil.example".parse().unwrap());
        let result = tokio_tungstenite::connect_async(req).await;
        assert!(
            result.is_err(),
            "handshake carrying a non-local Origin header must be rejected"
        );

        // Loopback origins (what React Native / Expo Go always send) are
        // admitted — a localhost-served page is as privileged as a local
        // process, which needs no Origin at all.
        let mut local = format!("ws://127.0.0.1:{port}")
            .into_client_request()
            .unwrap();
        local
            .headers_mut()
            .insert("origin", "http://localhost:8081".parse().unwrap());
        tokio_tungstenite::connect_async(local)
            .await
            .expect("loopback Origin must be admitted");

        // A native viewport (no Origin) still connects and can call RPC — the
        // reject must not be a blanket denial.
        let client = connect_ws(&format!("ws://127.0.0.1:{port}")).await.unwrap();
        let echoed = client.call("Echo", serde_json::json!("ok")).await.unwrap();
        assert_eq!(echoed, serde_json::json!("ok"));
    }

    #[tokio::test]
    async fn dropping_stream_receiver_cancels_server_side() {
        let client = memory_client(Arc::new(TestService));
        let items = client
            .subscribe("Never", serde_json::Value::Null)
            .await
            .unwrap();
        drop(items);
        // The next unary call still works — the dead stream didn't wedge the connection.
        let echoed = client.call("Echo", serde_json::json!(2)).await.unwrap();
        assert_eq!(echoed, serde_json::json!(2));
    }
}
