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
--
-- A buddy's computer is a sandbox of its own, named for it: on this PC
-- (src/kx/computers.rs, through cua) or in the cloud on Boat (src/kx/boat.rs,
-- with the person's key, which src/kx/keys.rs keeps and never gives back):
--   store.keys                  which keys are held ({ boat = true })
--   store.boats, store.read_boats(), store.boat_of(buddy), store.make_boat(buddy, then)
--   store.computer_ready        whether cua is installed to make one
--   store.computers             the sandboxes that are Keplorayx's, as last read
--   store.read_computers()      read again
--   store.computer_name(buddy)  what its sandbox is called
--   store.computer_of(buddy)    its sandbox now ({ name, status, … }), or nil
--   store.make_computer(buddy, image, then)   minutes; then(ok, why)
--   store.open_desktop(buddy)   its desktop, live, in the panel beside its chat

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

-- What's offered and usable, as one word: what changes when the list of
-- agents arrives (after the page may already show) or one signs in or out.
function M.agents_key(): string
	local parts = {}
	for _, a in M.agents() do
		table.insert(parts, `{a.harness}{if a.ok then "+" else "-"}`)
	end
	return `{table.concat(parts, ",")}/{#(chat_store.catalog:get().models or {})}`
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
	-- (Made with no agent of its own: each run says which it's on, so the
	-- chat follows whichever of the buddy's order was usable then.)
	sessions.create({ cwd = buddy.home, title = buddy.name }):after(function(id: any)
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

-- ── Its computer ─────────────────────────────────────────────────────────
M.computer_ready = ui.shared("kx.computer.ready", false)
M.computers = ui.state("kx_computers", {} :: { { name: string, status: string?, kind: string?, runtime: string? } })
M.computers_read = ui.state("kx_computers_read", false)

function M.read_computers()
	ui.call("kx.computer.list"):after(function(list: any)
		M.computers:set(if type(list) == "table" then list else {})
		M.computers_read:set(true)
	end, function()
		M.computers_read:set(true)
	end)
end

function M.computer_name(buddy: Buddy): string
	return `kx-{string.lower(buddy.id)}`
end

function M.computer_of(buddy: Buddy): { name: string, status: string?, kind: string?, runtime: string? }?
	local name = M.computer_name(buddy)
	for _, c in M.computers:get() or {} do
		if c.name == name then
			return c
		end
	end
	return nil
end

function M.make_computer(buddy: Buddy, image: string, after: (ok: boolean, why: string?) -> ())
	local name = M.computer_name(buddy)
	ui.call("kx.computer.create", { name = name, image = image }):after(function()
		-- (Said in its record, so its instructions tell it about it.)
		M.save({ id = buddy.id, name = buddy.name, computer = { kind = "local", name = name, image = image } }):after(function()
			M.read_computers()
			after(true, nil)
		end, function()
			M.read_computers()
			after(true, nil)
		end)
	end, function(why: any)
		M.read_computers()
		after(false, tostring(why))
	end)
end

-- ── Its computer in the cloud (Boat) ────────────────────────────────────
M.keys = ui.shared("kx.keys", {} :: { [string]: boolean })
M.boats = ui.state("kx_boats", {} :: { { id: string, name: string, state: string? } })
M.boats_read = ui.state("kx_boats_read", false)
M.boats_error = ui.state("kx_boats_error", nil :: string?)

function M.read_boats()
	ui.call("kx.boat.list"):after(function(list: any)
		M.boats:set(if type(list) == "table" then list else {})
		M.boats_error:set(nil)
		M.boats_read:set(true)
	end, function(why: any)
		M.boats_error:set(tostring(why))
		M.boats_read:set(true)
	end)
end

function M.boat_of(buddy: Buddy): { id: string, name: string, state: string? }?
	local name = M.computer_name(buddy)
	for _, c in M.boats:get() or {} do
		if c.name == name then
			return c
		end
	end
	return nil
end

function M.make_boat(buddy: Buddy, after: (ok: boolean, why: string?) -> ())
	local name = M.computer_name(buddy)
	ui.call("kx.boat.create", name):after(function(id: any)
		M.save({ id = buddy.id, name = buddy.name, computer = { kind = "boat", id = tostring(id), name = name } }):after(function()
			M.read_boats()
			after(true, nil)
		end, function()
			M.read_boats()
			after(true, nil)
		end)
	end, function(why: any)
		M.read_boats()
		after(false, tostring(why))
	end)
end

-- (Its record no longer says it has one: its instructions stop saying so.)
function M.forget_computer(buddy: Buddy)
	M.save({ id = buddy.id, name = buddy.name, computer = { kind = "none" } })
end

-- Its desktop in the panel beside its chat: a page, as any the panel's
-- browser shows (cua's own viewer, served on this machine alone; or Boat's,
-- at an address asked for afresh each time, good for ten minutes to open).
function M.open_desktop(buddy: Buddy, after: ((ok: boolean, why: string?) -> ())?)
	local done = after or function() end
	local boat = M.boat_of(buddy)
	local asked = if buddy.computer and buddy.computer.kind == "boat" and boat then ui.call("kx.boat.view", boat.id) else ui.call("kx.computer.view", M.computer_name(buddy))
	asked:after(function(url: any)
		if type(url) ~= "string" then
			done(false, "There was no address for its desktop.")
			return
		end
		M.selected:set(buddy.id)
		M.ensure_chat(buddy, function(chat: string?, err: string?)
			if not chat then
				done(false, err or "Its chat couldn't be made.")
				return
			end
			route.open_session(chat)
			local ok, panel = pcall(function(): any
				return (require :: any)("@app/screens/panel")
			end)
			if ok and panel and panel.open_url then
				-- (Once the chat is the one showing: the panel opens for it.)
				ui.after(120, function()
					panel.open_url(chat, url)
					done(true, nil)
				end)
			else
				done(false, "The panel isn't there to show it.")
			end
		end)
	end, function(why: any)
		done(false, tostring(why))
	end)
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
