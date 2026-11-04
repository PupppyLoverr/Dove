--!strict
-- The Ray bar: one pill for everything. Type to ask an agent; start with
-- `>` for a command, `@` to mention a project or an agent, `/` for the
-- agent's own commands. What matches arrives as a live list under the
-- pill; ↑ ↓ move through it, Enter takes it, Escape steps back.
--
-- On Home it sits alone, centred, with the project and agent it will use
-- as chips inside it. In a thread it is the composer: the same pill, its
-- send turning to Stop while the agent works.
--
--   Ray { place = "home" }     Ray { place = "thread" }

local kx = require("@kx")
local store = require("@kx/store")
local nav = require("@screens/nav")

local M = {}

export type Result = { id: string, icon: string, title: string, hint: string?, run: () -> () }

local text = ui.state("ray.text", "")
local active = ui.state("ray.active", 1)
local sending = ui.state("ray.sending", false)
local failure = ui.state("ray.failure", "")
-- The project and agent a new thread will use (Home): the last chosen.
M.project = ui.persist("ray.project", "")
M.agent = ui.persist("ray.agent", "")

local function mode(): string
	local t = text:get()
	local first = string.sub(t, 1, 1)
	if first == ">" then
		return "command"
	elseif first == "@" then
		return "mention"
	elseif first == "/" then
		return "slash"
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

local function project_name(id: string): string
	for _, p in store.projects:get() do
		if p.id == id then
			return p.name
		end
	end
	local first = store.projects:get()[1]
	return if first then first.name else "No project"
end

local function agent_label(id: string): string
	for _, a in store.agents:get() do
		if a.id == id then
			return a.label
		end
	end
	local first = store.agents:get()[1]
	return if first then first.label else "No agent"
end

local function agent_id(): string
	local id = M.agent:get()
	for _, a in store.agents:get() do
		if a.id == id then
			return id
		end
	end
	local first = store.agents:get()[1]
	return if first then first.id else ""
end

-- The commands `>` offers.
local function commands(): { Result }
	local list: { Result } = {
		{ id = "new", icon = "pen-new-square", title = "New thread", hint = "Ctrl N", run = function()
			nav.go("home")
			store.home()
		end },
		{ id = "inbox", icon = "inbox", title = "Open the Inbox", run = function()
			nav.go("inbox")
		end },
		{ id = "settings", icon = "settings-minimalistic", title = "Open Settings", hint = "Ctrl ,", run = function()
			nav.go("settings")
		end },
		{ id = "night", icon = "moon", title = "Theme: Night", run = function()
			ui.theme.set("orbit-night")
		end },
		{ id = "dawn", icon = "sun", title = "Theme: Dawn", run = function()
			ui.theme.set("orbit-dawn")
		end },
		{ id = "system", icon = "monitor", title = "Theme: follow the system", run = function()
			ui.theme.follow_system { dark = "orbit-night", light = "orbit-dawn" }
		end },
		{ id = "motion", icon = "waveform", title = "Reduce motion: on or off", run = function()
			kx.motion.reduce:set(not kx.motion.reduce:peek())
		end },
	}
	return list
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
		table.insert(list, { id = `a:{a.id}`, icon = "bot", title = a.label, hint = "agent", run = function()
			M.agent:set(a.id)
			text:set("")
		end })
	end
	return list
end

-- The agent's own slash commands: sent as typed, so whichever agent runs
-- the thread takes them.
local SLASH = { "compact", "clear", "review", "init", "model", "cost" }
local function slashes(): { Result }
	local list: { Result } = {}
	for _, s in SLASH do
		table.insert(list, { id = `s:{s}`, icon = "command", title = `/{s}`, run = function()
			text:set(`/{s} `)
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
	local source = if m == "command" then commands() elseif m == "mention" then mentions() else slashes()
	local out: { Result } = {}
	for _, r in source do
		if matches(r.title, q) then
			table.insert(out, r)
		end
	end
	return out
end)

local function send()
	local t = text:peek()
	if string.match(t, "^%s*$") or sending:peek() then
		return
	end
	sending:set(true)
	failure:set("")
	local project = M.project:peek()
	store.send(t, if project ~= "" then project else nil):after(function()
		text:set("")
		sending:set(false)
		nav.go("thread")
	end, function(why: any)
		failure:set(tostring(if type(why) == "table" then why.message else why))
		sending:set(false)
	end)
end

local function take()
	local list = results:peek()
	local r = list[math.clamp(active:peek(), 1, math.max(1, #list))]
	if r then
		if mode() ~= "slash" then
			text:set("")
		end
		r.run()
		active:set(1)
	end
end

local function on_key(e: any): boolean?
	local list = results:peek()
	if #list > 0 then
		if e.key == "ArrowDown" then
			active:set(active:peek() % #list + 1)
			return true
		elseif e.key == "ArrowUp" then
			active:set((active:peek() - 2) % #list + 1)
			return true
		elseif e.key == "Enter" and not e.shift then
			take()
			return true
		end
	end
	if e.key == "Escape" and text:peek() ~= "" then
		text:set("")
		return true
	end
	if e.key == "Enter" and not e.shift then
		send()
		return true
	end
	return nil
end

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

local PLACEHOLDER = {
	ask = "Ask an agent, or type > for commands",
	command = "Run a command",
	mention = "Mention a project or an agent",
	slash = "An agent command",
}

local MODE_ICON = { ask = "ray", command = "command", mention = "link", slash = "code" }

-- A chip inside the pill: what a new thread will use, and a menu to change it.
local function chip(icon: string, label: () -> string, open: State<boolean>, items: () -> { Node }): Node
	return kx.Menu {
		open = open,
		label = "Choose",
		build = function()
			return ui.column { w_full = true, table.unpack(items()) }
		end,
		anchor = kx.Button {
			tone = "ghost",
			size = "xs",
			shape = "pill",
			on_click = ui.toggle(open),
			kx.Icon { name = icon, size = 14 },
			ui.text { label, literal = true, truncate = true, max_w = 160 },
			kx.Icon { name = "alt-arrow-down", size = 12 },
		},
	}
end

local project_open = ui.state("ray.project_open", false)
local agent_open = ui.state("ray.agent_open", false)

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

function M.Ray(props: { place: "home" | "thread" }): Node
	local home = props.place == "home"
	local field_ref = ui.ref()
	-- Ctrl+K (or Search) asks for focus.
	ui.effect(function()
		if nav.ray_focus:get() > 0 then
			local el = field_ref:get()
			if el then
				el:focus()
			end
		end
	end)
	local running = function(): boolean
		local t = store.thread:get()
		return not home and t ~= nil and t.running
	end
	local chips = if home
		then ui.row {
			gap = "$space.1",
			items = "center",
			chip("folder", function()
				return project_name(M.project:get())
			end, project_open, function()
				local items: { Node } = {}
				for _, p in store.projects:peek() do
					table.insert(items, kx.MenuItem {
						p.name,
						icon = "folder",
						checked = function()
							return M.project:get() == p.id
						end,
						on_click = function()
							M.project:set(p.id)
						end,
					})
				end
				table.insert(items, kx.MenuItem { "Add a folder…", icon = "folder-add", on_click = pick_folder })
				return items
			end),
			chip("bot", function()
				return agent_label(agent_id())
			end, agent_open, function()
				local items: { Node } = {}
				for _, a in store.agents:peek() do
					table.insert(items, kx.MenuItem {
						a.label,
						checked = function()
							return agent_id() == a.id
						end,
						on_click = function()
							M.agent:set(a.id)
						end,
					})
				end
				return items
			end),
		}
		else nil
	local pill = kx.Pill {
		w_full = true,
		items = "end",
		ui.column {
			grow = 1,
			min_w = 0,
			gap = "$space.1",
			ui.row {
				w_full = true,
				items = "start",
				gap = "$space.2",
				pl = "$space.2",
				pt = "$kx.space.snug",
				ui.column {
					color = function()
						return if mode() == "ask" then "$text.tertiary" else "$accent"
					end,
					pt = "$kx.space.hair",
					kx.Icon {
						name = function()
							return MODE_ICON[mode()] or "ray"
						end,
						size = 16,
					},
				},
				ui.textarea {
					ref = field_ref,
					value = text,
					label = if home then "Ask" else "Reply",
					placeholder = function()
						return if home then PLACEHOLDER[mode()] else "Reply, or type > for commands"
					end,
					placeholder_color = "$text.tertiary",
					caret_color = "$accent",
					wrap = true,
					grow = 1,
					min_w = 0,
					-- As tall as its lines, from one to eight, then it scrolls.
					h = function(): number
						local t = text:get()
						local lines = 1
						for _ in string.gmatch(t, "\n") do
							lines += 1
						end
						-- A long line wraps: about 72 characters a line here.
						lines += math.floor(#t / 72)
						return math.clamp(lines, 1, 8) * 21 + 4
					end,
					p = 0,
					bg = "#00000000",
					border = 0,
					focus = { border = 0, outline = 0 },
					text_size = "$font.size.md",
					line_height = "$font.line.md",
					on_key = on_key,
				},
			},
			ui.row {
				w_full = true,
				items = "center",
				gap = "$space.1",
				pb = "$kx.space.hair",
				chips,
				ui.div { grow = 1 },
				ui.show(function()
					return failure:get() ~= ""
				end, ui.text {
					function()
						return failure:get()
					end,
					literal = true,
					text_size = "$font.size.xs",
					color = "$danger",
					truncate = true,
					max_w = 280,
				}),
			},
		},
		ui.show(
			running,
			kx.Button {
				tone = "secondary",
				shape = "pill",
				icon = true,
				size = "md",
				label = "Stop",
				on_click = store.interrupt,
				kx.Icon { name = "stop", size = 14 },
			},
			kx.Button {
				shape = "pill",
				icon = true,
				size = "md",
				label = "Send",
				busy = sending,
				disabled = function()
					return string.match(text:get(), "^%s*$") ~= nil
				end,
				on_click = send,
				kx.Icon { name = "arrow-up", size = 16 },
			}
		),
	}
	return ui.column {
		w_full = true,
		relative = true,
		pill,
		-- What matches, under the pill (above it in a thread, where the pill
		-- sits at the window's foot).
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
				mt = if home then "$space.2" else nil,
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

-- Home's three first prompts, for the project chosen.
function M.suggest(t: string)
	text:set(t)
	nav.ray_focus:set(nav.ray_focus:peek() + 1)
end

return M
