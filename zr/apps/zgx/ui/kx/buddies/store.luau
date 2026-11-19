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
export type Space = {
	id: string,
	name: string,
	display_name: string?,
	status: string?,
	kind: string?,
	image: string?,
	os: string?,
	runtime: string?,
	is_ours: boolean?,
}
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
	-- (In the sidebar: pinned to the grid at its top (the first six are,
	-- unless said otherwise), marked unread, or hidden from it.)
	pinned: boolean?,
	unread: boolean?,
	hidden: boolean?,
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
	-- (Renamed: its chat takes the new name too, so the title bar says it.)
	if buddy.id and type(buddy.name) == "string" and buddy.name ~= "" then
		local was = M.by_id(buddy.id)
		if was and was.chat and was.name ~= buddy.name then
			pcall(sessions.rename, was.chat, buddy.name)
		end
	end
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

-- (`mark` is the agent's mark's name for ds.AgentMark, nil when it has none:
-- a harness id is not one, and drawing it as one fails.)
-- An agent's name as people say it, for one this computer doesn't offer
-- (its id alone otherwise: "claude-code").
local AGENT_NAMES: { [string]: string } = {
	["claude-code"] = "Claude Code",
	claude_code = "Claude Code",
	codex = "Codex",
	["gemini-cli"] = "Gemini CLI",
	gemini = "Gemini CLI",
	opencode = "OpenCode",
	cursor = "Cursor",
	devin = "Devin",
	grok = "Grok",
	hermes = "Hermes",
	amp = "Amp",
	pi = "Pi",
}
function M.agent_name(harness: string): string
	return AGENT_NAMES[harness]
		or (string.gsub(string.gsub(harness, "[-_]", " "), "^%l", string.upper))
end

function M.primary_agent(buddy: Buddy): { harness: string, label: string, mark: string?, model: string?, ok: boolean }?
	local offered = M.agents()
	local by: { [string]: Agent } = {}
	for _, a in offered do
		by[a.harness] = a
	end
	local first = buddy.models and buddy.models[1]
	if first then
		local a = by[first.harness]
		return {
			harness = first.harness,
			label = if a then a.label else M.agent_name(first.harness),
			mark = if a then a.mark else nil,
			model = first.model,
			ok = if a then a.ok else false,
		}
	end
	for _, a in offered do
		if a.ok then
			return { harness = a.harness, label = a.label, mark = a.mark, model = nil, ok = true }
		end
	end
	return nil
end

-- Where a buddy sits in the sidebar: pinned (the grid at its top: the first
-- six are, until one is said otherwise), unread, hidden.
function M.is_pinned(buddy: Buddy, index: number): boolean
	if buddy.pinned ~= nil then
		return buddy.pinned
	end
	return index <= 6
end

function M.set_flag(id: string, flag: string, value: boolean?)
	local b = M.by_id(id)
	if not b then
		return
	end
	local record: { [string]: any } = { id = id, name = b.name }
	record[flag] = value
	M.save(record):after(function() end, function() end)
end

function M.toggle_pin(id: string, index: number)
	local b = M.by_id(id)
	if b then
		M.set_flag(id, "pinned", not M.is_pinned(b, index))
	end
end

function M.set_unread(id: string, unread: boolean)
	M.set_flag(id, "unread", unread)
end

function M.set_hidden(id: string, hidden: boolean)
	M.set_flag(id, "hidden", hidden)
end

function M.show_all_hidden()
	for _, b in M.list:peek() or {} do
		if b.hidden then
			M.set_flag(b.id, "hidden", false)
		end
	end
end

-- Whether the create-a-buddy dialog shows.
M.create_open = ui.state("kx_buddy_create_open", false)
-- Whether a buddy's chat shows its drawer (kx/buddies/chat_bar), and which
-- of its tabs; kept between launches.
M.inspector_open = ui.persist("kx_buddy_inspector", true)
-- Which of the drawer's tabs: "roster", "thread", "knowledge", or "settings"
-- (behind the gear).
M.inspector_tab = ui.persist("kx_buddy_inspector_tab", "thread")
-- Whether the "Create a routine" suggestion was put away.
M.routine_hint_hidden = ui.persist("kx_buddy_routine_hint_hidden", false)

function M.next_automation(buddy: Buddy): Automation?
	local soonest: Automation? = nil
	for _, a in M.automations:get() or {} do
		if a.buddy == buddy.id and a.enabled ~= false and a.next then
			if not soonest or (a.next :: number) < (soonest.next :: number) then
				soonest = a
			end
		end
	end
	return soonest
end

function M.last_run(buddy: Buddy): Run?
	for _, r in M.runs:get() or {} do
		if r.buddy == buddy.id then
			return r
		end
	end
	return nil
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

-- Each buddy's chat named after it (one renamed before chats followed, or
-- made from a template under the template's name, is put right once).
local renamed: { [string]: string } = {}
ui.effect(function()
	local titles: { [string]: string } = {}
	for _, c in sessions.now:get().chats or {} do
		titles[c.id] = c.title
	end
	for _, b in M.list:get() or {} do
		local t = b.chat and titles[b.chat]
		if t and b.name and b.name ~= "" and t ~= b.name and renamed[b.chat] ~= b.name then
			renamed[b.chat] = b.name
			pcall(sessions.rename, b.chat, b.name)
		end
	end
end)

function M.open(buddy: Buddy)
	M.selected:set(buddy.id)
	M.ensure_chat(buddy, function(chat: string?)
		if chat then
			route.open_session(chat)
		end
	end)
end

-- Its chat, with its drawer open on `tab` ("roster", "thread", "knowledge"
-- or "settings"): where a buddy is set up, as in Grok Bot.
function M.show(buddy: Buddy, tab: string?)
	M.inspector_open:set(true)
	M.inspector_tab:set(tab or "settings")
	M.open(buddy)
end

-- Each chat's status by its id, worked out once each time the chats change
-- (not once per face per frame).
local statuses = ui.computed(function(): { [string]: string }
	local by: { [string]: string } = {}
	for _, c in sessions.now:get().chats or {} do
		if c.id and c.status then
			by[c.id] = c.status
		end
	end
	return by
end)

function M.standing(buddy: Buddy): string?
	if not buddy.chat then
		return nil
	end
	return statuses:get()[buddy.chat]
end

-- ── What the back end offers beyond the record (src/kx/buddies.rs) ─────
-- A buddy saved as a template file where the person says: its path or nil.
function M.export(buddy: Buddy, after: (path: string?, why: string?) -> ())
	ui.call("kx.buddy.export", buddy.id):after(function(path: any)
		after(if type(path) == "string" then path else nil, nil)
	end, function(why: any)
		after(nil, tostring(why))
	end)
end

-- A template file the person picks, made into a buddy: its id or nil.
function M.import_template(after: (id: string?, why: string?) -> ())
	ui.call("kx.buddy.import_template"):after(function(r: any)
		after(if type(r) == "table" then r.id else nil, nil)
	end, function(why: any)
		after(nil, tostring(why))
	end)
end

-- The skills library: `{ name, description }` each.
M.skills = ui.state("kx_buddy_skills", {} :: { { name: string, description: string } })
function M.read_skills()
	-- (As above: a window that has not lent this has no skills to read.)
	local ok, promise = pcall(ui.call, "kx.skills.list")
	if not ok then
		return
	end
	promise:after(function(list: any)
		M.skills:set(if type(list) == "table" then list else {})
	end, function() end)
end

-- Whether a buddy has a skill: all of the library unless it names some.
function M.has_skill(buddy: Buddy, name: string): boolean
	local named = (buddy :: any).skills
	if type(named) ~= "table" then
		return true
	end
	return table.find(named, name) ~= nil
end

function M.set_skill(buddy: Buddy, name: string, on: boolean)
	local now: { string } = {}
	for _, s in M.skills:peek() do
		if (s.name == name and on) or (s.name ~= name and M.has_skill(buddy, s.name)) then
			table.insert(now, s.name)
		end
	end
	M.save({ id = buddy.id, name = buddy.name, skills = now })
end

-- What it knows, for the drawer's Knowledge tab: its own `MEMORY.md` and the
-- `ABOUT-YOU.md` every buddy shares (src/kx/buddies.rs keeps both).
M.memory = ui.state("kx_buddy_memory", { memory = "", about = "" } :: { memory: string, about: string })
function M.read_memory(id: string)
	-- (As above: a window that has not lent this has nothing to read.)
	local ok, promise = pcall(ui.call, "kx.buddy.memory", id)
	if not ok then
		return
	end
	promise:after(function(m: any)
		if type(m) == "table" then
			M.memory:set({ memory = tostring(m.memory or ""), about = tostring(m.about or "") })
		end
	end, function() end)
end

-- A save of what it knows (`which`: "mine" or "about"); did(ok, why).
function M.set_memory(id: string, which: string, text: string, did: (ok: boolean, why: string?) -> ())
	local ok, promise = pcall(ui.call, "kx.buddy.set_memory", { id = id, which = which, text = text })
	if not ok then
		did(false, "This window can't save it.")
		return
	end
	promise:after(function()
		M.read_memory(id)
		did(true, nil)
	end, function(why: any)
		did(false, tostring(why))
	end)
end

-- The computer every buddy shares: `{ id, name }` or nil.
M.shared_computer = ui.state("kx_buddy_shared_computer", nil :: { id: string, name: string? }?)
function M.read_shared_computer()
	-- (A window that has not lent this yet — a test host, a plugin's sandbox —
	-- has no such call to make: nothing to read, nothing to say.)
	local ok, promise = pcall(ui.call, "kx.computer.shared")
	if not ok then
		return
	end
	promise:after(function(space: any)
		M.shared_computer:set(if type(space) == "table" and space.id then space else nil)
	end, function() end)
end

function M.share_computer(space: { id: string, name: string? }?, after: ((ok: boolean, why: string?) -> ())?)
	ui.call("kx.computer.share", space or false):after(function()
		M.shared_computer:set(space)
		if after then
			after(true)
		end
	end, function(why: any)
		if after then
			after(false, tostring(why))
		end
	end)
end

-- ── Its computer (Cua Spaces) ───────────────────────────────────────────
M.computer_ready = ui.shared("kx.computer.ready", false)
M.computers = ui.state("kx_computers", {} :: { Space })
M.computers_read = ui.state("kx_computers_read", false)
M.computers_error = ui.state("kx_computers_error", nil :: string?)

function M.read_computers()
	ui.call("kx.computer.list"):after(function(list: any)
		M.computers:set(if type(list) == "table" then list else {})
		M.computers_error:set(nil)
		M.computers_read:set(true)
	end, function(why: any)
		M.computers_error:set(tostring(why))
		M.computers_read:set(true)
	end)
end

function M.computer_name(buddy: Buddy): string
	local shared = M.shared_computer:get()
	if buddy.computer and buddy.computer.kind == "shared" and shared then
		return shared.name or shared.id
	end
	if buddy.computer and buddy.computer.kind == "local" and buddy.computer.name and buddy.computer.name ~= "" then
		return buddy.computer.name
	end
	return `kx-{string.lower(buddy.id)}`
end

function M.computer_of(buddy: Buddy): Space?
	-- (An id kept from before counts only while it's attached: detached, its
	-- own `kx-` Space is the one, if there is one.)
	local kind = buddy.computer and buddy.computer.kind
	if kind == "none" then
		-- (Detached on purpose: none is its own until one is attached again.)
		return nil
	end
	local shared = M.shared_computer:get()
	local want_id = if kind == "local" then buddy.computer and buddy.computer.id elseif kind == "shared" and shared then shared.id else nil
	local want_name = M.computer_name(buddy)
	for _, c in M.computers:get() or {} do
		if (want_id and c.id == want_id) or c.name == want_name then
			return c
		end
	end
	return nil
end

function M.attach_space(buddy: Buddy, space: Space, after: ((ok: boolean, why: string?) -> ())?)
	local done = after or function() end
	M.save({
		id = buddy.id,
		name = buddy.name,
		computer = { kind = "local", id = space.id, name = space.name, image = space.image },
	}):after(function()
		M.read_computers()
		done(true, nil)
	end, function(why: any)
		done(false, tostring(why))
	end)
end

function M.make_computer(buddy: Buddy, image: string, after: (ok: boolean, why: string?) -> ())
	local name = `kx-{string.lower(buddy.id)}`
	ui.call("kx.computer.create", { name = name, image = image }):after(function(space_id: any)
		local id = if type(space_id) == "string" and space_id ~= "" then space_id else `local:{name}`
		-- (Said in its record, so its instructions tell it about it.)
		M.save({ id = buddy.id, name = buddy.name, computer = { kind = "local", id = id, name = name, image = image } }):after(function()
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
					panel.open_url(chat, url, nil, nil, { bare = true, title = `{buddy.name}'s desktop` })
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

-- (Read once the window starts; again when they change.)
M.read_shared_computer()
M.read_skills()

return M
