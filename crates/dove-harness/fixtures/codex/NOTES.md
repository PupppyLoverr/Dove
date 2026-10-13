# Codex app-server protocol notes (ground truth)

Captured 2026-10-12 on Windows 11 against a live `codex app-server` (stdio).

## Version and binary

- Codex CLI: `codex-cli 0.146.0`.
- Binary used: `C:\Users\ravth\.bun\bin\codex.exe`.
- The pnpm shim (`codex.cmd` / `codex.ps1` in `%LOCALAPPDATA%\pnpm\bin`) is broken on this machine: it points at `pnpm\global\v11\7df0-1a07addcdcf\node_modules\@openai\codex\bin\codex.js`, which does not exist. Dove should spawn the `.exe` directly, or the pnpm global install should be repaired.
- Schemas: `schema/` holds 275 files from `codex.exe app-server generate-json-schema --out <dir>` (no `--experimental`). Start with `schema/ServerNotification.json`, `schema/ServerRequest.json`, `schema/ClientRequest.json`, and `schema/v2/*Params.json` / `*Response.json`.
- Transcript: `turn.log` (136 lines, sent `>> ` and received `<< ` JSON only, paths and host name redacted).

## Working request shapes

Each line is one JSON object terminated by `\n`. No `"jsonrpc"` field.

Initialize (request, then response):
```
>> {"id":1,"method":"initialize","params":{"clientInfo":{"name":"dove","title":"Dove","version":"0.1.0"}}}
<< {"id":1,"result":{"userAgent":"dove/0.146.0 (...)","codexHome":"C:\\Users\\...\\.codex","platformFamily":"windows","platformOs":"windows"}}
```

Initialized (notification, no id, sent right after the initialize response):
```
>> {"method":"initialized"}
```
Sent every time. I did not test whether the server rejects requests without it.

thread/start:
```
>> {"id":2,"method":"thread/start","params":{"cwd":"C:\\path\\to\\repo","approvalPolicy":"on-request","sandbox":"workspace-write","model":"gpt-5.6-terra"}}
<< {"id":2,"result":{"thread":{"id":"01a12e12-36f6-...","ephemeral":false,"cwd":"...","status":{"type":"idle"},...},...}}
```
The `model` field is effectively required here (see gotcha 1). `approvalPolicy` accepts `untrusted`, `on-request`, `never`, or a granular object. `sandbox` accepts `read-only`, `workspace-write`, `danger-full-access`. `ephemeral: true` avoids writing a rollout file under `~/.codex/sessions`.

turn/start:
```
>> {"id":3,"method":"turn/start","params":{"threadId":"01a12e12-36f6-...","input":[{"type":"text","text":"Create a file named hello.txt ..."}]}}
<< {"id":3,"result":{"turn":{"id":"01a12e12-3c87-...","status":"inProgress","items":[],...}}}
```
Input items are a union. `{"type":"text","text":...}` works. `localImage` (`path`) and `image` (`url`) exist per schema, not tested. Optional per-turn overrides: `model`, `effort`, `summary`, `approvalPolicy`, `sandboxPolicy`, `cwd`, `outputSchema`.

turn/interrupt (tested in a separate probe, `interrupt_raw.log` in the scratchpad):
```
>> {"id":4,"method":"turn/interrupt","params":{"threadId":"...","turnId":"..."}}
<< {"id":4,"result":{}}
<< {"method":"turn/completed","params":{"threadId":"...","turn":{"id":"...","status":"interrupted",...}}}
```

Command approval (server-to-client request, has an `id`):
```
<< {"method":"item/commandExecution/requestApproval","id":0,"params":{"threadId":"...","turnId":"...","itemId":"exec-...","reason":"Do you want to allow ...?","command":"\"C:\\\\WINDOWS\\\\...\\\\powershell.exe\" -Command \"Set-Content ...\"","cwd":"...","availableDecisions":["accept",{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":[...]}},"cancel"],"proposedExecpolicyAmendment":[...]}}
>> {"id":0,"result":{"decision":"accept"}}
<< {"method":"serverRequest/resolved","params":{"threadId":"...","requestId":0}}
```
Reply with `result`, not `method`. Command decisions from the schema: `accept`, `acceptForSession`, `acceptWithExecpolicyAmendment`, `decline`, `cancel`, plus a network-policy variant. Use only values listed in `availableDecisions`.

File-change approval (`item/fileChange/requestApproval`) is not triggered in this run, because the model wrote the file with PowerShell. Per `schema/FileChangeRequestApprovalResponse.json`, the reply is `{"result":{"decision":"accept"|"acceptForSession"|"decline"|"cancel"}}`.

Legacy approvals `execCommandApproval` and `applyPatchApproval` use `ReviewDecision` (`schema/ExecCommandApprovalResponse.json`). Not exercised. My probe replies `"approved"`, which is unverified. Check the enum before relying on it.

Permissions request (`item/permissions/requestApproval`): reply shape from `schema/PermissionsRequestApprovalResponse.json`. Not exercised.

## Notifications observed (one example each, trimmed)

- `thread/started`: `{"method":"thread/started","params":{"thread":{"id":"01a12e12-...",...}}}`
- `turn/started`: `{"method":"turn/started","params":{"threadId":"...","turn":{"id":"01a12e12-3c87-...","status":"inProgress",...}}}`
- `item/started` (userMessage, reasoning, agentMessage, commandExecution): `{"method":"item/started","params":{"item":{"type":"commandExecution","id":"exec-506de654-...","command":"...","cwd":"...","source":"agent","status":"inProgress","aggregatedOutput":null,"exitCode":null,...},"threadId":"...","turnId":"...","startedAtMs":...}}`
- `item/agentMessage/delta` (48 seen): `{"method":"item/agentMessage/delta","params":{"threadId":"...","turnId":"...","itemId":"msg_...","delta":"I"}}`
- `item/commandExecution/outputDelta` (19 seen): `{"method":"item/commandExecution/outputDelta","params":{"threadId":"...","turnId":"...","itemId":"exec-...","delta":"\r\n"}}`. Output arrives as text chunks with `\r\n` line endings on Windows.
- `item/completed` (commandExecution): `{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"exec-...","status":"completed","aggregatedOutput":"...","exitCode":0,"durationMs":7047,...},"threadId":"...","turnId":"...","completedAtMs":...}}`
- `item/completed` (agentMessage): `{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"msg_...","text":"Created `hello.txt` containing `hi`...","phase":"final_answer"},...}}`. The `phase` field is `commentary` for intermediate messages and `final_answer` for the last one.
- `item/fileChange/*`: not observed in this run. `item/fileChange/outputDelta` and `item/fileChange/patchUpdated` exist per schema.
- Reasoning: `item/started` and `item/completed` with `"type":"reasoning"` were observed, with `summary` and `content` empty. No reasoning delta notifications arrived. Schema methods: `item/reasoning/summaryTextDelta`, `item/reasoning/summaryPartAdded`, `item/reasoning/textDelta`. Not observed. Summary text likely requires the `summary` param on turn/start.
- `thread/tokenUsage/updated` (6 seen): `{"method":"thread/tokenUsage/updated","params":{"threadId":"...","turnId":"...","tokenUsage":{"total":{"totalTokens":31400,"inputTokens":31141,"cachedInputTokens":15104,"outputTokens":...,"reasoningOutputTokens":...},"last":{...},"modelContextWindow":258400}}}`
- `turn/completed`: `{"method":"turn/completed","params":{"threadId":"...","turn":{"id":"...","status":"completed","error":null,"items":[...],"startedAt":...,"completedAt":...,"durationMs":56322}}}`. Statuses: `completed`, `failed` (with `turn.error.message`), `interrupted`.
- Other notifications seen: `thread/status/changed` (`status.type` = `idle`, `active`, `systemError`), `mcpServer/startupStatus/updated`, `account/rateLimits/updated`, `remoteControl/status/changed`, `serverRequest/resolved`, `warning`, `error` (with `willRetry`).

Where things live:
- Thread id: `result.thread.id` of the `thread/start` response. Also `params.threadId` in every thread-scoped notification, and `params.thread.id` in `thread/started`.
- Turn id: `result.turn.id` of the `turn/start` response, and `params.turn.id` in `turn/started` / `turn/completed`. Also `params.turnId` on item and delta notifications.
- Item id: `params.item.id` on item/started and item/completed. Deltas use `params.itemId`.
- Token usage: `params.tokenUsage.total` (cumulative for the thread) and `params.tokenUsage.last` (most recent model call), plus `modelContextWindow`. In the run, `total.totalTokens` went 15666 to 31400 to 108658 across calls.
- Rate limits: `account/rateLimits/updated`, `params.rateLimits.primary.usedPercent`.

## Gotchas

1. The default model in `C:\Users\ravth\.codex\config.toml` is `gpt-6-astra`. With this ChatGPT account the server returns HTTP 400 ("not supported when using Codex with a ChatGPT account") and `turn/completed` arrives with `status:"failed"`. The fix that worked was passing `"model":"gpt-5.6-terra"` in `thread/start`. Models listed as `visibility: list` in `C:\Users\ravth\.codex\models_cache.json` are: `gpt-5.6-terra`, `gpt-5.6-luna`. Dove should either pass `model` explicitly or read the model list first (`model/list`).
2. Each failing turn still returns a valid `turn/start` result immediately. Failure only shows in `turn/completed`, so check `turn.status`, not just the completion event.
3. The user's config starts an MCP server `wink` that timed out after 30 s (`MCP startup failed: timed out awaiting tools/list`). It delays the first turn and emits `mcpServer/startupStatus/updated` noise. It does not block the turn. Dove may want to set `-c mcp_servers.wink.enabled=false` for a clean environment, which is untested.
4. Stderr carries harmless noise (`failed to refresh available models: timeout waiting for child process`). Read stdout and stderr on separate threads, or stderr can fill and block the child.
5. Decode stdout as UTF-8 explicitly. Python's default on this machine is cp1252, and `models_cache.json` raises `UnicodeDecodeError` under it. Rust should use UTF-8 with lossy or strict handling. Windows paths come through JSON-escaped (`C:\\Users\\...`).
6. Command strings include the full `powershell.exe -Command "..."` invocation. The model chose PowerShell here, not `cmd`, so commands and output use Windows line endings.
7. The probe ran in a fresh `git init` directory under `%TEMP%`. Codex wrote a `[projects.'<dir>'] trust_level = "trusted"` entry into `C:\Users\ravth\.codex\config.toml` for that directory. Dove's own working directories will get the same kind of entry, which is a side effect on the user's config.
8. Each thread is written to `C:\Users\ravth\.codex\sessions\...\rollout-*.jsonl` unless `ephemeral: true` is set.
9. Every server message has `emittedAtMs`, and item events have `startedAtMs` / `completedAtMs`. Use these for timing, not wall clock.
10. Approval requests can arrive during `turn/start` before its response. Handle server requests in the same read loop as responses, not only after `turn/start` returns.
