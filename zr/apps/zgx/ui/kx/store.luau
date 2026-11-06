--!strict
-- Keplorayx's island, launcher and widgets read threads as Keplorayx's own
-- store had them; here, from Zeron's engine through what every window gets
-- (src/sessions.rs `sessions`, src/chat.rs `chat.*`): the threads, the one
-- open in this window, its messages and the reply being written, what an
-- agent waits on, and the calls to open, send, stop and answer.

export type Thread = {
	id: string,
	title: string,
	status: string,
	harness: string,
	harness_id: string,
	project: string?,
	updated: number?,
}
export type Open = { id: string, title: string, harness: string, running: boolean, status: string }
export type Approval = { request_id: string, tool: string, summary: string, questions: { any } }
export type Waiting = {
	thread_id: string,
	title: string,
	harness: string,
	harness_id: string,
	request_id: string,
	tool: string,
	summary: string,
}

local sessions = ui.shared("sessions", {} :: any)
local asks = ui.shared("chat.asks", {} :: any)
local session = ui.shared("chat.session", nil :: any)
local messages = ui.shared("chat.messages", {} :: any)
local draft = ui.shared("chat.draft", { id = "", text = "" } :: any)

local store = {}
store.fixture = (ui.shared("dev.fixture", "") :: any):peek() :: string
-- (Keplorayx's screens read their fixture's data here; Zeron's app runs
-- them on the engine's own, its mock agent for tries: ZERON_ZGX_MOCK=1.)
store.fixture_data = {} :: { [string]: any }

-- An agent's display name by Zeron's harness id.
local NAMES: { [string]: string } = {
	claude = "Claude Code",
	claude_code = "Claude Code",
	codex = "Codex",
	cursor = "Cursor",
	opencode = "OpenCode",
	pi = "Pi",
	gemini = "Gemini",
	grok = "Grok",
	devin = "Devin",
	hermes = "Hermes",
	antigravity = "Antigravity",
	muse = "Muse",
}
-- (Zeron's sidebar states, as Keplorayx's.)
local STATUS: { [string]: string } = { working = "running", input = "needs_you", failed = "failed", done = "done" }

store.threads = ui.computed(function(): { Thread }
	local s = sessions:get() or {}
	local out: { Thread } = {}
	for _, c in s.chats or {} do
		local h = c.harness or c.agent or ""
		table.insert(out, {
			id = c.id,
			title = c.title or "",
			status = STATUS[c.status or ""] or "idle",
			harness = NAMES[h] or h,
			harness_id = c.agent or h,
			project = c.project,
			updated = c.updated,
		})
	end
	table.sort(out, function(a, b)
		return (a.updated or 0) > (b.updated or 0)
	end)
	return out
end)

store.thread = ui.computed(function(): Open?
	local o = session:get()
	if not o then
		return nil
	end
	local h = o.harness or ""
	return { id = o.id, title = o.title or "", harness = NAMES[h] or h, running = o.running == true, status = "" }
end)

store.messages = messages
store.draft = draft

-- What the open thread waits on (an approval), as Keplorayx's.
store.approval = ui.computed(function(): Approval?
	local o = session:get()
	local a = o and o.ask
	if not (a and a.kind == "approval") then
		return nil
	end
	local ap = a.approval or {}
	return {
		request_id = a.request,
		tool = ap.title or "Allow this?",
		summary = ap.command or ap.detail or "",
		questions = {},
	}
end)

-- Every thread waiting on you (Zeron's `chat.asks`), with its title.
store.attention = ui.computed(function(): { Waiting }
	local waiting = asks:get() or {}
	local out: { Waiting } = {}
	for _, t in store.threads:get() do
		local kind = waiting[t.id]
		if kind then
			table.insert(out, {
				thread_id = t.id,
				title = t.title,
				harness = t.harness,
				harness_id = t.harness_id,
				request_id = "",
				tool = if kind == "question" then "a question" else "something",
				summary = "",
			})
		end
	end
	return out
end)

function store.open(id: string)
	pcall(ui.invoke, "chat.open", id)
end

function store.home()
	pcall(ui.invoke, "chat.open", nil)
end

-- A message's id, as Zeron's composer makes one (screens/chat/store).
local function new_id(): string
	local hex = "0123456789abcdef"
	local out = {}
	for i = 1, 32 do
		local n = math.random(1, 16)
		if i == 13 then
			n = 5
		elseif i == 17 then
			n = 9 + (n % 4)
		end
		table.insert(out, string.sub(hex, n, n))
	end
	local s = table.concat(out)
	return `{string.sub(s, 1, 8)}-{string.sub(s, 9, 12)}-{string.sub(s, 13, 16)}-{string.sub(s, 17, 20)}-{string.sub(s, 21, 32)}`
end

-- A new thread with `text` as its first message, as Zeron's composer starts
-- one: made in the project (or on this computer), then opened here and sent.
-- A promise of nothing; it fails with the reason.
function store.start(text: string, with: { [string]: any }?): any
	local w = with or {}
	local project = if w.project and w.project ~= "" then w.project else nil
	local here = (sessions:peek() or {}).here
	return {
		after = function(self: any, done: (any) -> (), fail: ((any) -> ())?)
			ui.call("sessions.create", { project = project, device = if project then nil else here }):after(function(made: any)
				local id = if type(made) == "table" then made.id else made
				if type(id) ~= "string" then
					if fail then
						fail("The thread couldn't be made")
					end
					return
				end
				store.open(id)
				ui.call("chat.send", {
					session = id,
					fresh = true,
					text = text,
					message_id = new_id(),
					project = project,
					device = if project then nil else here,
					model = w.model,
					effort = w.effort,
				}):after(function()
					done(id)
				end, function(e: any)
					if fail then
						fail(e)
					end
				end)
			end, function(e: any)
				if fail then
					fail(e)
				end
			end)
			return self
		end,
	}
end

-- A message to the open thread (queued while it works); with `with` (a new
-- thread's project, model, effort), a new thread.
function store.send(text: string, with: { [string]: any }?): any
	if with then
		return store.start(text, with)
	end
	local o = session:peek()
	if not o then
		return ui.sleep(0):after(function()
			error("No thread is open")
		end)
	end
	return ui.call("chat.send", { session = o.id, text = text })
end

function store.interrupt()
	local o = session:peek()
	if o then
		pcall(ui.call, "chat.interrupt", o.id)
	end
end

-- An approval answered: "allow_once" | "allow_always" | "deny".
function store.decide(request_id: string, decision: string, _answers: any?)
	local o = session:peek()
	if not o then
		return
	end
	local d = if decision == "deny" then "deny" elseif decision == "allow_always" then "always" else "once"
	pcall(ui.call, "chat.respond", { id = o.id, request = request_id, decision = d })
end

return store
