# ACP notes (Dove harness)

Protocol: Agent Client Protocol, JSON-RPC 2.0 over stdio, one JSON object per line (newline-delimited).
Spec: https://agentclientprotocol.com (protocol/initialization, session-setup, prompt-turn, tool-calls, file-system; schema).
Observed agent: OpenCode 1.18.32 (registry lists 1.19.0). Probe: `scratchpad/acp_probe.py`, transcript `opencode.log`.

## ACP launch command per agent

| Agent | ACP launch command | Source | Verified end-to-end |
|---|---|---|---|
| opencode | `opencode acp` | ACP registry (`binary.args: ["acp"]`); `opencode --help` lists `acp` | Yes, `stopReason=end_turn`, `hello.txt` = `hi` |
| devin | `devin acp` | ACP registry; `devin acp --help` ("Run as an ACP server over stdio") | No, not needed after opencode succeeded |
| grok | `grok agent stdio` (npx: `@xai-official/grok@1.0.52 agent stdio`) | Registry; `grok agent --help` lists `stdio` | No |
| droid | `droid exec --output-format acp-daemon` (npx: `droid@0.238.0`) | Registry only; not in `droid --help` | No |
| cursor-agent | `cursor-agent acp` | Registry (`args: ["acp"]`); not confirmed in `--help` | No |
| amp | No ACP subcommand in `amp`. Use the separate wrapper `amp-acp` (github.com/tao12345666333/amp-acp, v0.10.0) | Registry; `amp --help` has no acp | No |

## Working JSON shapes (opencode, verified)

Client → agent (all requests carry `"jsonrpc":"2.0"`):

```json
{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":false},"clientInfo":{"name":"dove-acp-probe","title":"Dove ACP probe","version":"0.0.1"}}}
{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":"<absolute repo path>","mcpServers":[]}}
{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{"sessionId":"ses_...","prompt":[{"type":"text","text":"Create a file hello.txt containing hi."}]}}
{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"ses_..."}}
```

Agent → client responses:

```json
{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"promptCapabilities":{"image":true,"embeddedContext":true},"mcpCapabilities":{"http":true,"sse":true},"sessionCapabilities":{"close":{},"fork":{},"list":{},"resume":{}}},"authMethods":[{"id":"opencode-login","name":"Login with opencode","description":"Run `opencode auth login` in the terminal"}],"agentInfo":{"name":"OpenCode","version":"1.18.32"}}}
{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_ed1ec66ebffeJWT2oJh4q0Wi58","configOptions":[...]}}
{"jsonrpc":"2.0","id":2,"result":{"stopReason":"end_turn","usage":{"inputTokens":144,"outputTokens":3,"totalTokens":13587,"cachedReadTokens":13440},"_meta":{}}}
```

stopReason values (spec): `end_turn`, `max_tokens`, `max_turn_requests`, `refusal`, `cancelled`.

Permission request (agent → client, has `id`) and response, per spec. Not triggered in this run, so the shape is spec-only:

```json
{"jsonrpc":"2.0","id":5,"method":"session/request_permission","params":{"sessionId":"...","toolCall":{"toolCallId":"call_001"},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}
{"jsonrpc":"2.0","id":5,"result":{"outcome":{"outcome":"selected","optionId":"allow-once"}}}
{"jsonrpc":"2.0","id":5,"result":{"outcome":{"outcome":"cancelled"}}}
```

File system (agent → client, spec; not triggered by opencode, see gotchas):

```json
{"jsonrpc":"2.0","id":7,"method":"fs/read_text_file","params":{"sessionId":"...","path":"C:\\abs\\file.txt","line":1,"limit":20}}
{"jsonrpc":"2.0","id":7,"result":{"content":"file text"}}
{"jsonrpc":"2.0","id":8,"method":"fs/write_text_file","params":{"sessionId":"...","path":"C:\\abs\\hello.txt","content":"hi"}}
{"jsonrpc":"2.0","id":8,"result":{}}
```

`line` (1-based) and `limit` are optional. The agent must only call these if the client advertised `fs.readTextFile` / `fs.writeTextFile`.

session/cancel: notification, no id. After sending it, the client answers pending `session/request_permission` with `outcome: cancelled`, marks open tool calls as cancelled, and waits for the `session/prompt` response with `stopReason: "cancelled"`.

## session/update variants observed

Envelope: `{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"...","update":{"sessionUpdate":"<variant>", ...}}}` (no `id`).

- `available_commands_update` (sent right after `session/prompt`): `{"sessionUpdate":"available_commands_update","availableCommands":[{"name":"...","description":"..."}]}`
- `agent_thought_chunk` (streamed): `{"sessionUpdate":"agent_thought_chunk","messageId":"prt_...","content":{"type":"text","text":"Simple"}}`
- `tool_call`: `{"sessionUpdate":"tool_call","toolCallId":"call_00_...","title":"write","kind":"edit","status":"pending","locations":[],"rawInput":{}}`
- `tool_call_update` (in progress): `{"sessionUpdate":"tool_call_update","toolCallId":"call_00_...","status":"in_progress","kind":"edit","title":"write","locations":[{"path":"~/.../hello.txt"}],"rawInput":{"filePath":"..."}}`
- `tool_call_update` (done): `{"sessionUpdate":"tool_call_update","toolCallId":"call_00_...","status":"completed","title":"hello.txt","content":[{"type":"content","content":{"type":"text","text":"Wrote file successfully."}}],"rawOutput":{"output":"Wrote file successfully.","metadata":{...}}}`
- `agent_message_chunk`: `{"sessionUpdate":"agent_message_chunk","messageId":"msg_...","content":{"type":"text","text":"Done."}}`
- `usage_update`: `{"sessionUpdate":"usage_update","used":13584,"size":200000,"cost":{"amount":0,"currency":"USD"}}`

Spec-only (not seen in this run): `plan`, `user_message_chunk` (replayed on `session/load`), `current_mode_update`, `config_option_update`, `session_info_update`, `notice`, `compaction_update`.

## Gotchas

- Tool calls arrive before the file exists: the first `tool_call` has `title: "write"`, `rawInput: {}`, and `locations: []`. Fill the UI from later `tool_call_update` messages, which carry the real title, paths, and `rawOutput`.
- Opencode wrote `hello.txt` with its own tool, not via `fs/write_text_file`. Clients must not assume the agent calls the fs methods, and must handle both paths.
- No `session/request_permission` was sent for this prompt, so the permission flow is untested end-to-end.
- Opencode advertises an `authMethods` entry (`opencode-login`) but `session/new` succeeded without logging in. Treat authMethods as informational until `session/new` actually errors.
- Notifications (`session/update`) stream before the `session/prompt` response. The response with `id` is the only end-of-turn marker.
- Paths are Windows-style with backslashes, JSON-escaped as `\\`. Pass absolute paths.
- stdout must be UTF-8. On a Windows console, printing the transcript with cp1252 raised `UnicodeEncodeError`. Use `sys.stdout.reconfigure(encoding="utf-8")` or write to a file.
- On Windows, `opencode` resolves to a sh shim (`/c/nvm4w/nodejs/opencode`). Use `shutil.which`; if it resolves to `.cmd`, run it through `cmd /c`.
- `session/new` returns a large `configOptions` array (model list). Ignore it or parse it lazily.
- `session/prompt` result includes `usage` and `_meta` beyond the spec.
- Redacted fixture: home path replaced with `~`, temp repo with `<REPO>`, emails and tokens scrubbed. The redaction regexes must handle JSON-escaped backslashes; a naive single-backslash replace missed them.
