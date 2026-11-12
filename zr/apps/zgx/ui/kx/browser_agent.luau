--!strict
-- Agents in the in-app browser. What an agent asks of a page (the
-- engine's `browser_*` tools) arrives in `kx.browser.requests`
-- (src/kx/browser.rs: a list of `{ id, op, chat, who, agent, args }`,
-- oldest first); each is done here, on the browser tab beside its chat,
-- and answered with `kx.browser.reply`.
--
-- The tab is the chat's own panel's (screens/panel/state): the browser tab
-- in front there, else its last, else one made by `open`. Its page is the
-- host's (`ui.web`, by the tab's id, as screens/panel/browser makes it), so
-- it is the page the person sees, sign-ins and all.
--
-- On the page, ui/kx/browser_agent_script (`window.__kxAgent`) does the
-- step: each agent's cursor moves there first, and the page wears a blue
-- shimmer while any agent is on it. Agents are numbered by tab as they
-- arrive ("Agent 1", "Agent 2", …): a chat's own number on its own tab,
-- more when several chats (or one chat's several cursors) share a tab.
-- Steps of different agents run at once; nothing here queues them.
--
-- A page that goes somewhere else loses the script with it: the tab's
-- ticker puts it back (cursors where the agents are counted, the shimmer
-- on) while any agent has worked there in the last while.

local state = require("@app/screens/panel/state")
local api = require("@app/screens/panel/browser_api")
local data = require("@app/screens/panel/browser_data")
local route = require("@app/screens/shell/route")
local view = require("@app/screens/shell/view")
local sessions = require("@account/sessions")
local SCRIPT: string = require("@app/kx/browser_agent_script")

local M = {}

local requests = ui.shared("kx.browser.requests", {} :: { any })

-- How long a step's page has to answer, how often it's asked, how long a
-- page has to load, and how long an agent that has gone quiet keeps its
-- cursor on the page.
local STEP_MS, ASK_MS, LOAD_MS, QUIET_MS = 30000, 90, 25000, 20000

type Agent = { n: number, at: number }
type Held = { web: any, agents: { [string]: Agent }, next: number, stop: (() -> ())? }
-- The tabs agents are on, by tab id.
local held: { [string]: Held } = {}
-- The requests begun (each once), by id.
local begun: { [string]: boolean } = {}

local function reply(id: string, json: string?, err: string?)
	pcall(ui.invoke, "kx.browser.reply", { id = id, json = json, error = err })
end

-- A question to a page, answered once: by the page, or (it never does:
-- its tab has never been shown, so it has no view to live in) by giving up.
--
-- One at a time: with several agents on one tab asking at once, answers
-- went missing (two of three steps never heard theirs). So questions wait
-- in a line, the next asked as the one before is answered.
local EVAL_MS = 4000
type Asked = { web: any, js: string, answer: (value: any, err: string?) -> () }
-- (One line for them all: a page's handle isn't the same value each time
-- it's asked for, so there's no telling two pages' lines apart by it.)
local the_line: { busy: boolean, waiting: { Asked } } = { busy = false, waiting = {} }
local pump: () -> ()
pump = function()
	local line = the_line
	if line.busy then
		return
	end
	local q = table.remove(line.waiting, 1)
	if not q then
		return
	end
	line.busy = true
	local answered = false
	local function once(value: any, err: string?)
		if answered then
			return
		end
		answered = true
		line.busy = false
		-- (Its answer, then the next in line: an answer that asks again
		-- joins the line's end.)
		local ok, why = pcall(q.answer, value, err)
		if not ok then
			print(`kx-browser: {tostring(why)}`)
		end
		pump()
	end
	local ok = pcall(function()
		q.web:eval(q.js, once)
	end)
	if not ok then
		once(nil, "the page can't be asked")
		return
	end
	ui.after(EVAL_MS, function()
		once(nil, "the page didn't answer")
	end)
end
local function eval(web: any, js: string, answer: (value: any, err: string?) -> ())
	table.insert(the_line.waiting, { web = web, js = js, answer = answer })
	pump()
end

local function has_page(web: any): boolean
	local u = web.url
	return type(u) == "string" and u ~= "" and u ~= "about:blank"
end

-- The agents on a tab now, as its page's script takes them.
local function roster(h: Held): string
	local list = {}
	for key, a in h.agents do
		table.insert(list, { key = key, n = a.n })
	end
	table.sort(list, function(a: any, b: any): boolean
		return a.n < b.n
	end)
	-- (An empty Luau table encodes as an object: say an empty list.)
	return if #list == 0 then "[]" else ui.json.encode(list)
end

-- The page's script there and its cursors as counted, then `after` (with
-- whether the page answered at all).
local function ensure(h: Held, after: ((ok: boolean) -> ())?)
	local list = roster(h)
	eval(h.web, `window.__kxAgent ? window.__kxAgent.sync({list}) : "__none"`, function(value: any, err: string?)
		if err then
			if after then
				after(false)
			end
			return
		end
		if value == "__none" then
			h.web:run(`{SCRIPT}\n;window.__kxAgent.sync({list});`)
		end
		if after then
			after(true)
		end
	end)
end

-- A tab's ticker: while agents are on it, their cursors and the shimmer
-- are kept there (through a page going elsewhere); quiet ones leave; with
-- none left, the shimmer goes and the ticker with it.
local function tick(id: string)
	local h = held[id]
	if not h then
		return
	end
	local now = ui.now()
	for key, a in h.agents do
		if now - a.at > QUIET_MS then
			h.agents[key] = nil
		end
	end
	ensure(h)
	if next(h.agents) == nil then
		if h.stop then
			h.stop()
		end
		held[id] = nil
	end
end

local function hold(id: string, web: any): Held
	local h = held[id]
	if not h then
		h = { web = web, agents = {}, next = 1 }
		held[id] = h
	end
	h.web = web
	if not h.stop then
		h.stop = ui.every(1000, function()
			tick(id)
		end)
	end
	return h
end

-- An agent's cursor on a tab: its number there (the next, as it arrives).
local function agent_on(h: Held, key: string): number
	local a = h.agents[key]
	if not a then
		a = { n = h.next, at = ui.now() }
		h.next += 1
		h.agents[key] = a
	end
	a.at = ui.now()
	return a.n
end

-- The page's web for a tab, made as the panel's browser makes it (the same
-- name and cookie profile), so the two are one page.
local function web_of(tab: string): (any, string?)
	local profile = data.profile()
	if not profile then
		return nil, "Keplorayx's browser isn't ready yet (it's still reading whose sign-ins its pages keep). Try again in a moment."
	end
	local ok, web = pcall(function(): any
		return api.web(`panel-browser-{tab}`, {
			profile = profile,
			messages = true,
			new_windows = true,
			inspectable = data.inspectable:peek(),
		})
	end)
	if not ok or not web then
		return nil, "Keplorayx couldn't make a page for the browser tab."
	end
	return web, nil
end

-- The chat a side chat or subagent is under (nil: it's under none).
local function parent_of(chat: string): string?
	for parent, list in sessions.now:peek().children or {} do
		for _, child in list do
			if child.id == chat then
				return parent
			end
		end
	end
	return nil
end

-- A chat's browser tab: the one in front, else its last; nil for none.
local function browser_tab(key: string): string?
	local s = state.sessions:peek()[key]
	if not s then
		return nil
	end
	local last: string? = nil
	for _, t in s.tabs do
		if t.kind == "browser" then
			if t.id == s.active then
				return t.id
			end
			last = t.id
		end
	end
	return last
end

-- Until the page has loaded (its document there and settled), then
-- `after(ok)`. `from`: the address it's leaving, so the old page isn't
-- taken for the new one.
local function loaded(web: any, from: string?, after: (ok: boolean) -> ())
	local started = ui.now()
	local settled = 0
	local function look()
		local waited = ui.now() - started
		if waited > LOAD_MS then
			after(false)
			return
		end
		eval(web, "document.readyState + '|' + location.href", function(value: any, err: string?)
			local ready, href = "", ""
			if not err and type(value) == "string" then
				local bar = string.find(value, "|", 1, true)
				if bar then
					ready, href = string.sub(value, 1, bar - 1), string.sub(value, bar + 1)
				end
			end
			-- (Still the page it's leaving: it has a while to move on; a page
			-- loaded again, or one that only changed its fragment, never does.)
			local moved = href ~= "" and href ~= "about:blank" and (from == nil or href ~= from or waited > 1500)
			if moved and ready == "complete" then
				after(true)
				return
			elseif moved and ready == "interactive" then
				-- (Its document is there; its pictures may take long: enough.)
				settled += 1
				if settled >= 8 then
					after(true)
					return
				end
			end
			ui.after(150, look)
		end)
	end
	ui.after(200, look)
end

-- Where the page is, as JSON text, with `extra` (JSON text of an object)
-- folded in.
local function whereabouts(web: any, extra: string, after: (json: string?) -> ())
	eval(web, `JSON.stringify(Object.assign({extra}, \{ url: location.href, title: document.title }))`, function(value: any, err: string?)
		after(if not err and type(value) == "string" then value else nil)
	end)
end

-- One step on the page, its answer taken as it comes. A page that left
-- under a click (a link followed) is answered once it has loaded.
local function step(h: Held, req: any, key: string, n: number)
	local id: string = req.id
	local op: string = req.op
	local q = ui.json.encode({ id = id, op = op, key = key, n = n, args = req.args })
	local started = ui.now()
	local again = false
	-- (How many times running the page has said it never began this step.)
	local lost = 0
	local from = h.web.url
	-- (`extra`: an object, as the page's script reads it; `plain`: what's
	-- said if the new page doesn't answer, JSON text.)
	local function finish_moved(extra: string, plain: string)
		loaded(h.web, from, function()
			ensure(h, function()
				whereabouts(h.web, extra, function(json: string?)
					reply(id, json or plain, nil)
				end)
			end)
		end)
	end
	local ask: () -> ()
	local function start()
		ensure(h, function(ok: boolean)
			if not ok then
				reply(id, nil, "The page didn't answer. It may still be loading, or the chat's browser tab isn't showing in Keplorayx: open it there, then try again.")
				return
			end
			h.web:run(`window.__kxAgent && window.__kxAgent.op({q});`)
			ui.after(ASK_MS, ask)
		end)
	end
	ask = function()
		if ui.now() - started > STEP_MS then
			reply(id, nil, "The page took too long over that step.")
			return
		end
		eval(h.web, `window.__kxAgent ? window.__kxAgent.take("{id}") : "__gone"`, function(value: any, err: string?)
			if err then
				ui.after(ASK_MS * 2, ask)
				return
			end
			-- (The page's script is there but never began this step: the
			-- page went elsewhere and the script was put back on the new one,
			-- by the tab's ticker or another agent's step. Said a few times
			-- running, so a step only just sent isn't taken for a lost one.)
			if value == "__lost" then
				lost += 1
				if lost < 4 then
					ui.after(ASK_MS, ask)
					return
				end
				value = "__gone"
			else
				lost = 0
			end
			if value == "__gone" then
				-- (The page went elsewhere mid-step. A click, a key or typing
				-- that sent it there is done; anything else is asked again.)
				if op == "click" or op == "key" or op == "type" then
					local said = `\{ "did": "{op}", "navigated": true }`
					finish_moved(said, said)
				elseif not again then
					again = true
					lost = 0
					loaded(h.web, nil, function()
						start()
					end)
				else
					reply(id, nil, "The page kept moving on before the step was done.")
				end
				return
			end
			if type(value) ~= "string" then
				ui.after(ASK_MS, ask)
				return
			end
			-- (A click that follows a link answers before the page leaves:
			-- a moment on, where it went is said with it.)
			if op == "click" or op == "key" or op == "type" then
				ui.after(450, function()
					eval(h.web, `window.__kxAgent ? "here" : "__gone"`, function(still: any)
						if still == "__gone" or h.web.url ~= from then
							finish_moved(`Object.assign({value}, \{ navigated: true })`, value)
						else
							reply(id, value, nil)
						end
					end)
				end)
				return
			end
			reply(id, value, nil)
		end)
	end
	start()
end

local function run(req: any)
	local id: string = req.id
	local op: string = req.op
	local args: any = req.args or {}
	-- Whose tab: the chat asked for; else the caller's, or the chat it's
	-- under (a subagent or a side chat browses in its parent's tab, beside
	-- it, a cursor of its own: one page, the one the person is watching);
	-- else the one open.
	local chat: any = req.chat
	local asked = type(args.chat) == "string" and args.chat ~= ""
	if type(chat) == "string" and chat ~= "" and not asked then
		chat = parent_of(chat) or chat
	end
	if type(chat) ~= "string" or chat == "" then
		chat = route.chat:peek().session
	end
	if type(chat) ~= "string" or chat == "" then
		reply(id, nil, "No chat is open in Keplorayx for the browser to sit beside. Open one there, or call from a chat.")
		return
	end
	local key = `chat:{chat}`
	local showing = route.chat:peek().session == chat
	local tab = browser_tab(key)

	local url: string? = nil
	if op == "open" then
		local resolved, why = api.resolve(tostring(args.url or ""), data.engine_url())
		if not resolved then
			reply(id, nil, `That isn't an address the browser can open{if why then ": " .. tostring(why) else "."}`)
			return
		end
		url = resolved
		if not tab or args.new_tab == true then
			tab = state.open(key, "browser", { url = resolved })
		else
			state.set_url(key, tab, resolved)
			state.activate(key, tab)
		end
		-- (Beside the chat showing, its panel opens: the person watches.)
		if showing then
			pcall(view.set_files, true)
		end
	end
	if not tab then
		reply(id, nil, "There's no browser tab beside this chat yet: call browser_open first.")
		return
	end
	local web, why = web_of(tab)
	if not web then
		reply(id, nil, why)
		return
	end
	local h = hold(tab, web)
	-- Whose cursor: the caller's chat and its number.
	local who = `{if type(req.who) == "string" and req.who ~= "" then req.who else chat}#{tostring(req.agent or 1)}`

	if op == "done" then
		h.agents[who] = nil
		ensure(h)
		reply(id, `\{ "done": true }`, nil)
		return
	end
	local n = agent_on(h, who)

	if op == "open" or op == "go" then
		local from = web.url
		if op == "open" then
			web:load(url :: string)
		else
			local to = tostring(args.to or "")
			if to == "back" then
				web:back()
			elseif to == "forward" then
				web:forward()
			elseif to == "reload" then
				web:reload()
			else
				reply(id, nil, "Say where: back, forward or reload.")
				return
			end
		end
		loaded(web, if op == "go" and args.to == "reload" then nil else from, function(ok: boolean)
			ensure(h, function()
				whereabouts(web, if ok then `\{ "loaded": true }` else `\{ "loaded": false, "note": "The page is still loading; browser_read reads what's there so far." }`, function(json: string?)
					if json then
						reply(id, json, nil)
					else
						reply(id, nil, "The page didn't answer after loading. If the chat's browser tab isn't showing in Keplorayx, open it there, then try browser_read.")
					end
				end)
			end)
		end)
		return
	end

	if not has_page(web) then
		reply(id, nil, "No page is open in the browser tab yet: call browser_open first.")
		return
	end
	if op == "read" or op == "click" or op == "type" or op == "key" or op == "scroll" or op == "wait" then
		step(h, req, who, n)
		return
	end
	reply(id, nil, `The browser has no step "{op}".`)
end

-- Each request begun once, as it arrives (outside the effect: what a step
-- reads isn't what wakes this).
ui.effect(function()
	local list = requests:get() or {}
	local waiting: { [string]: boolean } = {}
	for _, req in list do
		if type(req) == "table" and type(req.id) == "string" then
			waiting[req.id] = true
			if not begun[req.id] then
				begun[req.id] = true
				ui.after(0, function()
					local ok, err = pcall(run, req)
					if not ok then
						reply(req.id, nil, `Keplorayx's browser hit an error on that step: {tostring(err)}`)
					end
				end)
			end
		end
	end
	-- (Answered ones are forgotten once they've left the list.)
	for id in begun do
		if not waiting[id] then
			begun[id] = nil
		end
	end
end)

-- How many agents are on browser pages now (for whoever shows it).
function M.agents(): number
	local n = 0
	for _, h in held do
		for _ in h.agents do
			n += 1
		end
	end
	return n
end

return M
