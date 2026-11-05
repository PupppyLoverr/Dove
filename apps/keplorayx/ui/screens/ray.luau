--!strict
-- The Ray bar: Zeron's composer (ds/composer, vendored; MIT, see
-- THIRD_PARTY_NOTICES.md) in Keplorayx's words. The draft on top, + to
-- attach, the microphone and Send beneath it; under the pill, what the
-- message will use: the access (with plan mode), where it works (the
-- project, or a thread's branch), and the model with its reasoning (Zeron's
-- model picker: a tab for each agent, its models, an effort slider). What
-- an agent asks (an approval, a question) opens in the pill itself, the
-- draft then the answer.
--
-- Keplorayx's own on top: start with `>` for a command or `@` for a
-- project or an agent, and what matches floats by the pill; Enter takes
-- the first, a click any.
--
--   Ray { place = "home" }     Ray { place = "thread" }

local kx = require("@kx")
local Composer = require("@ds/composer")
local store = require("@kx/store")
local nav = require("@screens/nav")

local M = {}

export type Result = { id: string, icon: string, title: string, hint: string?, run: () -> () }

local text = ui.state("ray.text", "")
local active = ui.state("ray.active", 1)
local sending = ui.state("ray.sending", false)
local notice = ui.state("ray.notice", nil :: any)
local dictating = ui.state("ray.dictating", false)
-- The project a new thread will use (Home): the last chosen.
M.project = ui.persist("ray.project", "")
-- The agent of the model chosen (kept for the screens that ask which).
M.agent = ui.persist("ray.agent", "")
-- The model ("<agent>:<model>") and how hard it thinks.
M.model = ui.persist("ray.model", nil :: string?)
M.effort = ui.persist("ray.effort", nil :: string?)
-- How much a new thread's agent may do before it asks: "supervised" (ask
-- first), "edits" (edit files freely, ask before commands), "full"; and
-- plan mode (explore and propose before changing anything).
M.access = ui.persist("ray.access_mode", "supervised" :: string?)
M.plan = ui.persist("ray.plan", false)
-- The teammate a new thread runs as (Team › Start a thread), for the next
-- thread only.
M.persona = ui.state("ray.persona", nil :: { id: string, name: string }?)

-- Each agent's mark, as the design system names them.
local MARK: { [string]: string } = {
	claude_code = "claude",
	codex = "openai",
	cursor = "cursor",
	grok = "grok",
	devin = "devin",
	open_code = "opencode",
	antigravity = "antigravity",
	pi = "pi",
	hermes = "hermes",
}

-- The picker's tabs: every installed agent.
local harnesses = ui.computed(function(): { any }
	local out: { any } = {}
	for _, a in store.agents:get() do
		table.insert(out, { id = a.id, label = a.label, mark = MARK[a.id] or "claude" })
	end
	return out
end)

-- Its models, as the engine lists them (src/panels.rs `kx.models`).
local models = ui.state("ray.models", {} :: { any })
store.call("kx.models", nil, {}):after(function(list: any)
	models:set(list or {})
	-- No model chosen yet: the first agent's first.
	if M.model:peek() == nil and list and list[1] then
		M.model:set(list[1].value)
		M.effort:set(list[1].effort)
	end
end, function() end)

local function agent_id(): string
	local m = M.model:get()
	local agent = if m then string.match(m, "^([^:]+):") else nil
	if agent then
		return agent
	end
	local first = store.agents:get()[1]
	return if first then first.id else ""
end

local function mode(): string
	local first = string.sub(text:get(), 1, 1)
	if first == ">" then
		return "command"
	elseif first == "@" then
		return "mention"
	end
	return "ask"
end

local function query(): string
	local t = text:get()
	return string.lower(if mode() == "ask" then t else string.sub(t, 2))
end

local function matches(title: string, q: string): boolean
	return q == "" or string.find(string.lower(title), q, 1, true) ~= nil
end

-- The commands `>` offers.
local function commands(): { Result }
	local function panel(tab: string): () -> ()
		return function()
			require("@screens/panel").show(tab)
		end
	end
	return {
		{ id = "new", icon = "pen-new-square", title = "New thread", hint = "Ctrl+N", run = function()
			nav.go("home")
			store.home()
		end },
		{ id = "inbox", icon = "inbox", title = "Open the Inbox", hint = "Ctrl+I", run = function()
			nav.go("inbox")
		end },
		{ id = "team", icon = "users-group", title = "Open Team", run = function()
			nav.go("team")
		end },
		{ id = "files", icon = "folder-with-files", title = "Browse the project's files", hint = "Ctrl+P", run = panel("files") },
		{ id = "browser", icon = "global", title = "Open the browser", hint = "Ctrl+Shift+B", run = panel("browser") },
		{ id = "changes", icon = "file-diff", title = "Review the changes", hint = "Ctrl+J", run = panel("changes") },
		{ id = "terminal", icon = "terminal", title = "Open a terminal", hint = "Ctrl+`", run = panel("terminal") },
		{ id = "history", icon = "git-branch", title = "Show the project's history", run = panel("history") },
		{ id = "pulls", icon = "pull-request", title = "Show pull requests", run = panel("pulls") },
		{ id = "settings", icon = "settings-minimalistic", title = "Open Settings", hint = "Ctrl+,", run = function()
			nav.go("settings")
		end },
		{ id = "night", icon = "moon", title = "Theme: Night", run = function()
			ui.theme.set("orbit-night")
		end },
		{ id = "dawn", icon = "sun", title = "Theme: Dawn", run = function()
			ui.theme.set("orbit-dawn")
		end },
		{ id = "motion", icon = "waveform", title = "Reduce motion: on or off", run = function()
			kx.motion.reduce:set(not kx.motion.reduce:peek())
		end },
	}
end

local function mentions(): { Result }
	local list: { Result } = {}
	for _, p in store.projects:get() do
		table.insert(list, { id = `p:{p.id}`, icon = "folder", title = p.name, hint = "project", run = function()
			M.project:set(p.id)
			text:set("")
		end })
	end
	for _, a in store.agents:get() do
		table.insert(list, { id = `a:{a.id}`, icon = kx.MARKS[a.id] or "bot", title = a.label, hint = "agent", run = function()
			for _, m in models:peek() do
				if m.harness == a.id then
					M.model:set(m.value)
					M.effort:set(m.effort)
					break
				end
			end
			text:set("")
		end })
	end
	return list
end

local results = ui.computed(function(): { Result }
	local m = mode()
	if m == "ask" then
		return {}
	end
	local q = query()
	local out: { Result } = {}
	for _, r in if m == "command" then commands() else mentions() do
		if matches(r.title, q) then
			table.insert(out, r)
		end
	end
	return out
end)

local function take()
	local list = results:peek()
	local r = list[math.clamp(active:peek(), 1, math.max(1, #list))]
	if r then
		text:set("")
		r.run()
		active:set(1)
	end
end

local function send()
	if #results:peek() > 0 then
		take()
		return
	end
	local t = text:peek()
	if string.match(t, "^%s*$") or sending:peek() then
		return
	end
	sending:set(true)
	notice:set(nil)
	local project = M.project:peek()
	local persona = M.persona:peek()
	store.send(t, {
		project = if project ~= "" then project else nil,
		harness = agent_id(),
		model = M.model:peek(),
		effort = M.effort:peek(),
		access = if M.plan:peek() then "plan" else M.access:peek(),
		persona = if persona then persona.id else nil,
	}):after(function()
		text:set("")
		sending:set(false)
		M.persona:set(nil)
		nav.go("thread")
	end, function(why: any)
		notice:set({ tone = "danger", text = tostring(if type(why) == "table" then why.message else why), dismissable = true })
		sending:set(false)
	end)
end

local function pick_folder()
	ui.task(function()
		local folder: any = ui.open_folder({ title = "Choose a project folder" }):await()
		if not folder then
			return
		end
		-- The handle hides its path: a file in it shows where it is.
		local entries = folder:list("", { limit = 1, files = true }):await()
		local path: string? = nil
		local first = entries and entries[1]
		if first and first.file then
			local full: string = first.file.path
			path = string.sub(full, 1, #full - #first.path - 1)
		end
		if not path then
			kx.toast { title = "That folder is empty", body = "Add a file to it first", kind = "error" }
			return
		end
		local id = store.add_project(path):await()
		if type(id) == "string" then
			M.project:set(id)
		end
		kx.toast { title = `Added {folder.name}` }
	end)
end

M.pick_folder = pick_folder

-- Files attached to the next message: their paths, put into it as
-- mentions the agent reads (every harness reads a path it's given).
local function attach()
	ui.task(function()
		local files: any = ui.open_files({ title = "Attach files", multiple = true }):await()
		if not files then
			return
		end
		local parts: { string } = {}
		for _, f in files do
			table.insert(parts, `@{f.path}`)
		end
		if #parts > 0 then
			local t = text:peek()
			text:set((if t == "" then "" else t .. " ") .. table.concat(parts, " ") .. " ")
		end
	end)
end

-- What the open thread waits on, as the composer's panels show it.
local approval = ui.computed(function(): any
	local a = store.approval:get()
	if not a or #a.questions > 0 then
		return nil
	end
	return { title = `Run {a.tool}?`, command = a.summary }
end)
local question = ui.computed(function(): any
	local a = store.approval:get()
	if not a or #a.questions == 0 then
		return nil
	end
	local q = a.questions[1]
	local options = {}
	for _, o in q.options do
		table.insert(options, { value = o, label = o })
	end
	return { header = "Question", text = q.text, options = options, page = 1, pages = #a.questions }
end)
local chosen = ui.state("ray.chosen", {} :: { [string]: boolean })

local function decide(decision: string, answers: { { string } }?)
	local a = store.approval:peek()
	if a then
		store.decide(a.request_id, decision, answers)
		chosen:set({})
	end
end

-- The projects a new thread can work in.
local project_list = ui.computed(function(): { any }
	local out: { any } = {}
	for _, p in store.projects:get() do
		table.insert(out, { value = p.id, label = p.name })
	end
	return out
end)
local project_choice = ui.state("ray.project_choice", nil :: string?)
ui.effect(function()
	local id = M.project:get()
	if id == "" then
		local first = store.projects:get()[1]
		id = if first then first.id else ""
	end
	project_choice:set(if id ~= "" then id else nil)
end)
ui.effect(function()
	local id = project_choice:get()
	if id and id ~= M.project:peek() then
		M.project:set(id)
	end
end)

-- The chosen project's branch, read as the project changes: a new thread
-- starts in the project's own checkout, on it (the chip's one choice).
local branch = ui.state("ray.branch", nil :: string?)
local branches = ui.state("ray.branches", {} :: { any })
local checkout = ui.state("ray.checkout", "local" :: string?)
ui.effect(function()
	local id = project_choice:get()
	if not id then
		return
	end
	store.call("kx.branch", { project = id }, "main"):after(function(b: any)
		local name = if type(b) == "string" then b else nil
		branch:set(name)
		branches:set(if name then { { value = name, label = name } } else {})
	end, function() end)
end)

-- The open thread's place: its branch, fixed (a thread can't move).
local function fixed(): any
	local t = store.thread:get()
	if not t then
		return nil
	end
	for _, th in store.threads:get() do
		if th.id == t.id then
			return {
				name = if th.branch and th.branch ~= "" then th.branch else nil,
				worktree = th.worktree == true,
				project = th.project,
			}
		end
	end
	return nil
end

local focus = ui.state("ray.focus", 0)

local function result_row(r: Result, i: number): Node
	return kx.Row {
		size = "sm",
		selected = function()
			return active:get() == i
		end,
		on_click = function()
			active:set(i)
			take()
		end,
		on_hover = function(on: boolean)
			if on then
				active:set(i)
			end
		end,
		ui.column { color = "$text.secondary", kx.Icon {
			name = function()
				return r.icon
			end,
			size = 16,
		} },
		ui.text {
			function()
				return r.title
			end,
			literal = true,
			grow = 1,
			truncate = true,
		},
		ui.text {
			function()
				return r.hint or ""
			end,
			literal = true,
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		},
	}
end

function M.Ray(props: { place: "home" | "thread" }): Node
	local home = props.place == "home"
	-- Ctrl+K (or Search) asks for focus.
	ui.effect(function()
		if nav.ray_focus:get() > 0 then
			focus:set(focus:peek() + 1)
		end
	end)
	local running = ui.computed(function(): boolean
		local t = store.thread:get()
		return not home and t ~= nil and t.running
	end)
	local composer = Composer {
		value = text,
		placeholder = if home then "Ask anything, @ to mention, > for commands" else "Reply, or > for commands",
		keep = `kx.ray.{props.place}`,
		expanded = home,
		menus_below = home,
		-- Over Home's sky the pill's glass runs on under its chips.
		over_art = function(): boolean
			return home and require("@screens/home").sky:get() ~= "none"
		end,
		running = running,
		access = M.access,
		plan = M.plan,
		model = M.model,
		effort = M.effort,
		harnesses = harnesses,
		models = models,
		project = if home then project_choice else nil,
		projects = if home then project_list else nil,
		on_new_project = if home then pick_folder else nil,
		branch = if home then branch else nil,
		branches = if home then branches else nil,
		checkout = if home then checkout else nil,
		branch_fixed = if home then nil else fixed,
		on_send = send,
		on_stop = store.interrupt,
		on_attach = attach,
		approval = approval,
		on_approve = function(scope: string)
			decide(if scope == "always" then "allow_always" else "allow_once")
		end,
		on_deny = function()
			decide("deny")
		end,
		question = question,
		chosen = chosen,
		on_pick = function(value: string)
			local next = table.clone(chosen:peek())
			next[value] = if next[value] then nil else true
			chosen:set(next)
		end,
		on_answer = function(answer: any)
			local picked: { string } = table.clone(answer.values or {})
			if answer.text and answer.text ~= "" then
				table.insert(picked, answer.text)
			end
			decide("allow_once", { picked })
			text:set("")
		end,
		on_close_question = function()
			decide("deny")
		end,
		notice = notice,
		on_dismiss_notice = function()
			notice:set(nil)
		end,
		dictating = dictating,
		on_dictate = function()
			-- Windows' own voice typing (Win+H) into the draft.
			focus:set(focus:peek() + 1)
			pcall(ui.invoke, "kx.dictate")
		end,
		focus = focus,
		mention_enabled = function(_trigger: string): boolean
			return false
		end,
	}
	return ui.column {
		w_full = true,
		relative = true,
		gap = "$space.1",
		-- Who it runs as, when a teammate was chosen.
		ui.show(function()
			return home and M.persona:get() ~= nil
		end, ui.row {
			px = "$space.2",
			gap = "$space.1",
			items = "center",
			ui.button {
				h = 24,
				px = "$kx.space.tight",
				gap = "$space.1",
				items = "center",
				rounded = "$radius.md",
				bg = "$kx.secondary.bg",
				cursor = "pointer",
				tooltip = "Runs as this teammate. Click to clear.",
				on_click = function()
					M.persona:set(nil)
				end,
				kx.Icon { name = "teammate", size = 13 },
				ui.text {
					function()
						local p = M.persona:get()
						return if p then `as {p.name}` else ""
					end,
					literal = true,
					text_size = "$font.size.sm",
				},
				kx.Icon { name = "close", size = 11 },
			},
		}),
		composer,
		-- What `>` and `@` match, floating by the pill (under it on Home, over
		-- it in a thread).
		ui.show(
			function()
				return #results:get() > 0
			end,
			kx.Float {
				absolute = true,
				left = 0,
				right = 0,
				top = if home then "100%" else nil,
				bottom = if home then nil else "100%",
				mt = if home then "$space.1" else nil,
				mb = if home then nil else "$space.2",
				z = 10,
				max_h = 320,
				ui.each(results, result_row, {
					key = function(r: Result)
						return r.id
					end,
					w_full = true,
					gap = "$kx.space.hair",
				}),
			}
		),
	}
end

-- Home's first prompts, for the project chosen.
function M.suggest(t: string)
	text:set(t)
	nav.ray_focus:set(nav.ray_focus:peek() + 1)
end

return M
