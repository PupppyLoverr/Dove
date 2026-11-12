--!strict
-- Buddies, as the scripts read and change them. A buddy is a named bot with
-- a job (src/kx/buddies.rs keeps them, their folders and their automations):
--
--   store.list:get()           { Buddy }
--   store.automations:get()    { Automation } (each with `next`, unix seconds)
--   store.runs:get()           { Run }, newest first
--   store.selected             the buddy whose page shows (its id)
--   store.by_id(id), store.by_chat(chat)
--   store.is_chat(chat)        whether a chat is a buddy's (the sessions' list
--                              leaves those out: they're under Buddies)
--   store.save(buddy)          a promise of its id
--   store.remove(id)
--   store.agents()             what a buddy can run on now: every agent the
--                              composer offers, each with whether it's usable
--                              and, if not, why
--   store.pick(buddy)          the first of its order that's usable (else the
--                              first usable at all): harness, model, a note
--                              saying so when it isn't its first choice
--   store.ensure_chat(buddy, then)   its chat's id, made if it has none
--   store.open(buddy)          its chat, in the window
--   store.standing(buddy)      "working", "input", "failed", "done", or nil
--                              (idle), as its chat stands
--   store.send(buddy, prompt, then)  a message to it by the app (an
--                              automation's): then(ok, note, used)

local sessions = require("@account/sessions")
local agents = require("@account/agents")
local chat_store = require("@app/screens/chat/store")
local route = require("@app/screens/shell/route")

export type Step = { harness: string, model: string? }
export type Look = { shape: string?, color: string?, face: string? }
export type Buddy = {
	id: string,
	name: string,
	role: string?,
	instructions: string?,
	models: { Step }?,
	folder: string?,
	permissions: string?,
	look: Look?,
	chat: string?,
	home: string?,
	computer: { kind: string?, id: string?, name: string? }?,
	created: number?,
}
export type Automation = {
	id: string,
	buddy: string,
	title: string?,
	prompt: string,
	kind: string,
	minutes: number?,
	time: string?,
	days: { number }?,
	at: number?,
	enabled: boolean?,
	cap: number?,
	last_run: number?,
	next: number?,
}
export type Run = { run: string, automation: string?, buddy: string, title: string?, at: number, status: string, note: string?, chat: string? }
export type Agent = { harness: string, label: string, mark: string?, ok: boolean, why: string? }

local M = {}

M.list = ui.shared("kx.buddies", {} :: { Buddy })
M.automations = ui.shared("kx.automations", {} :: { Automation })
M.runs = ui.shared("kx.buddy.runs", {} :: { Run })
M.selected = ui.persist("kx_buddy_selected", nil :: string?)

function M.by_id(id: string?): Buddy?
	for _, b in M.list:get() or {} do
		if b.id == id then
			return b
		end
	end
	return nil
end

function M.by_chat(chat: string?): Buddy?
	if not chat then
		return nil
	end
	for _, b in M.list:get() or {} do
		if b.chat == chat then
			return b
		end
	end
	return nil
end

function M.is_chat(chat: string?): boolean
	return M.by_chat(chat) ~= nil
end

function M.save(buddy: { [string]: any }): any
	return ui.call("kx.buddy.save", buddy)
end

function M.remove(id: string)
	if M.selected:peek() == id then
		M.selected:set(nil)
	end
	pcall(ui.invoke, "kx.buddy.remove", id)
end

-- Whether an agent can take a turn now: installed, on, signed in, and its
-- account in use under its limits. (An agent with no account listed, one
-- that keeps its own sign-in, is taken as usable: the engine says otherwise
-- when it runs.)
local function usable(id: string): (boolean, string?)
	for _, a in agents.now:get().list or {} do
		if a.id == id then
			if not a.installed then
				return false, "not installed"
			elseif not a.enabled then
				return false, "switched off"
			end
			local active: any = nil
			for _, acc in a.accounts or {} do
				if acc.active then
					active = acc
				end
			end
			if #(a.accounts or {}) > 0 and not active then
				return false, "signed out"
			end
			if active and ((active.session or 0) >= 100 or (active.week or 0) >= 100) then
				return false, "at its limit"
			end
			return true, nil
		end
	end
	return true, nil
end

function M.agents(): { Agent }
	local out: { Agent } = {}
	for _, h in chat_store.catalog:get().harnesses or {} do
		local ok, why = usable(h.id)
		table.insert(out, { harness = h.id, label = h.label, mark = h.mark :: any, ok = ok, why = why })
	end
	return out
end

-- An agent's models, as the composer offers them.
function M.models(harness: string): { { value: string, label: string } }
	local out = {}
	for _, m in chat_store.catalog:get().models or {} do
		if m.harness == harness then
			table.insert(out, { value = m.value, label = m.label })
		end
	end
	return out
end

function M.pick(buddy: Buddy): (string?, string?, string?)
	local offered = M.agents()
	local by: { [string]: Agent } = {}
	for _, a in offered do
		by[a.harness] = a
	end
	local order = buddy.models or {}
	for i, step in order do
		local a = by[step.harness]
		if a and a.ok then
			local first = order[1]
			local note = if i > 1 and first
				then `{(by[first.harness] or { label = first.harness } :: any).label} is {(by[first.harness] or {} :: any).why or "not available"}, so this ran on {a.label}.`
				else nil
			return step.harness, step.model, note
		end
	end
	-- (None of its own is usable, or it names none: whichever is.)
	for _, a in offered do
		if a.ok then
			local note = if #order > 0 then `None of its agents is available now, so this ran on {a.label}.` else nil
			return a.harness, nil, note
		end
	end
	return nil, nil, "No agent is signed in and under its limit."
end

-- Its chat, known to the engine still (not one deleted since).
local function has_chat(chat: string?): boolean
	if not chat then
		return false
	end
	local now = sessions.now:peek()
	for _, c in now.chats do
		if c.id == chat then
			return true
		end
	end
	for _, c in now.archived or {} do
		if c.id == chat then
			return true
		end
	end
	return false
end

function M.ensure_chat(buddy: Buddy, after: (chat: string?, err: string?) -> ())
	if has_chat(buddy.chat) then
		after(buddy.chat, nil)
		return
	end
	local harness, model = M.pick(buddy)
	sessions.create({
		cwd = buddy.home,
		title = buddy.name,
		config = if harness then { harness = harness, model = model } else nil,
	}):after(function(id: any)
		if type(id) ~= "string" then
			after(nil, "The engine made no chat.")
			return
		end
		M.save({ id = buddy.id, name = buddy.name, chat = id }):after(function()
			after(id, nil)
		end, function()
			after(id, nil)
		end)
	end, function(why: any)
		after(nil, tostring(why))
	end)
end

function M.open(buddy: Buddy)
	M.selected:set(buddy.id)
	M.ensure_chat(buddy, function(chat: string?)
		if chat then
			route.open_session(chat)
		end
	end)
end

function M.standing(buddy: Buddy): string?
	if not buddy.chat then
		return nil
	end
	for _, c in sessions.now:get().chats do
		if c.id == buddy.chat then
			return c.status
		end
	end
	return nil
end

function M.send(buddy: Buddy, prompt: string, after: (ok: boolean, note: string?, used: { chat: string?, harness: string?, model: string? }) -> ())
	local harness, model, note = M.pick(buddy)
	if not harness then
		after(false, note, {})
		return
	end
	M.ensure_chat(buddy, function(chat: string?, err: string?)
		if not chat then
			after(false, err or "Its chat couldn't be made.", {})
			return
		end
		ui.call("kx.buddy.send", { chat = chat, prompt = prompt, harness = harness, model = model, cwd = buddy.home }):after(function()
			after(true, note, { chat = chat, harness = harness, model = model })
		end, function(why: any)
			after(false, tostring(why), { chat = chat, harness = harness, model = model })
		end)
	end)
end

return M
