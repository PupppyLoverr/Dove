--!strict
-- The sidebar, as Zeron's (its screens/shell/sidebar.luau over ds/sidebar,
-- both MIT, vendored): the design system's sidebar on our threads, with
-- Zeron's view options (Organize and its order, Sort, Show: branch, pull
-- request, agent, project icon, where, archived, the project filter, empty
-- projects, keep what's active), your own places (Pinned and sections,
-- dragged to and between), the archived shelf, a thread's menu as Zeron's
-- (Rename in its row, Pin, Copy, Move to, Archive, Delete), a project's
-- (New thread here, Show in folder, Remove), and the jump keys (hold Ctrl:
-- each row's number; Ctrl+1–9 opens it). Folded away (Ctrl+B), its rail.
--
-- Keplorayx's own: our agents' marks, each project's tile on a hue of its
-- own, Constellation and Team at its foot beside Settings.

local ds = require("@ds")
local Sidebar = require("@ds/sidebar")
local kx = require("@kx")
local store = require("@kx/store")
local nav = require("@screens/nav")

type Thread = store.Thread

local M = {}

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

local fixture = store.fixture ~= ""
local function kept<T>(key: string, default: T): State<T>
	return if fixture then ui.state(key, default) else ui.persist(key, default)
end

-- ── Its view: Zeron's options, kept ────────────────────────────────────

local shared = {
	compact = nav.sidebar_compact,
	organize = nav.sidebar_organize,
	organize_order = kept("kx.sb.organize_order", { "project", "device" }),
	sort = kept("kx.sb.sort", "updated"),
	width = nav.sidebar_width,
	details = {
		branch = nav.sidebar_details.branch,
		pr = nav.sidebar_details.pr,
		harness = nav.sidebar_details.harness,
		project_icon = nav.sidebar_details.project_icon,
		location = nav.sidebar_details.location,
		archived = kept("kx.sb.show_archived", false),
		project_filter = kept("kx.sb.show_project_filter", false),
		empty_projects = kept("kx.sb.show_empty_projects", true),
		keep_active = kept("kx.sb.keep_active", true),
	},
	expanded = ui.state("kx.sb.expanded", {} :: { [string]: boolean }),
	project = kept("kx.sb.project", nil :: string?),
	-- Where threads are put by hand: Pinned and your sections, in order.
	places = kept("kx.sb.places", { pinned = {}, sections = {} } :: any),
	selected = ui.state("sidebar.selected", nil :: string?),
	offset = ui.state("kx.sb.offset", 0),
	folded = nav.folded,
}
M.options = shared

-- (Threads pinned before, in the store or on this device, join Pinned once.)
local merged_old_pins = false
ui.effect(function()
	local threads = store.threads:get()
	if merged_old_pins or #threads == 0 then
		return
	end
	merged_old_pins = true
	local places = table.clone(shared.places:peek())
	local pinned = table.clone(places.pinned or {})
	local old = nav.pinned:peek()
	for _, t in threads do
		if (t.pinned or old[t.id]) and not table.find(pinned, t.id) then
			table.insert(pinned, t.id)
		end
	end
	places.pinned = pinned
	places.sections = places.sections or {}
	shared.places:set(places)
end)

-- ── Rows ───────────────────────────────────────────────────────────────

-- A thread's state, as its corner says it: nothing for one at rest.
local function kind(t: Thread): string?
	if t.status == "needs_you" then
		return "approval"
	elseif t.status == "running" then
		return "working"
	elseif t.status == "failed" then
		return if t.unread then "failed" else nil
	elseif t.unread and (t.status == "done" or t.status == "finished") then
		return "done"
	end
	return nil
end

local function session(t: any): any
	return {
		id = t.id,
		title = t.title,
		status = if t.status then kind(t) else nil,
		-- (ms in the store; the sidebar counts in seconds.)
		time = math.floor((t.updated or 0) / 1000),
		agent = MARK[t.harness_id],
		branch = if t.branch and t.branch ~= "" then t.branch else nil,
		created = math.floor((t.updated or 0) / 1000),
	}
end

-- Each project's tile on a hue of its own, from its name (the same every
-- launch): Orbit's cool hues, a warm one or two for contrast.
local HUES = { "violet", "indigo", "blue", "sky", "teal", "emerald", "rose", "amber" }
local function hue_of(name: string): string
	local h = 0
	for i = 1, #name do
		h = (h * 31 + string.byte(name, i)) % 2147483647
	end
	return HUES[h % #HUES + 1]
end

-- Every project and its threads (the component moves those placed by hand
-- to Pinned or a section itself); narrowed to one by the project filter.
local projects = ui.computed(function(): { any }
	local only = if shared.details.project_filter:get() then shared.project:get() else nil
	local by: { [string]: any } = {}
	local out: { any } = {}
	for _, p in store.projects:get() do
		if not only or p.id == only then
			local g = { id = p.id, name = p.name, hue = hue_of(p.name), sessions = {} }
			by[p.id] = g
			table.insert(out, g)
		end
	end
	for _, t in store.threads:get() do
		local g = by[t.project_id]
		if not g and not only then
			g = { id = t.project_id, name = t.project, hue = hue_of(t.project), sessions = {} }
			by[t.project_id] = g
			table.insert(out, g)
		end
		if g then
			table.insert(g.sessions, session(t))
		end
	end
	for _, g in out do
		g.empty = #g.sessions == 0
	end
	return out
end)

-- The archived shelf: read while it shows, again as one is archived.
local shelf = ui.state("kx.sb.shelf", {} :: { any })
local function read_shelf()
	if fixture then
		return
	end
	store.call("kx.archived", nil, {}):after(function(list: any)
		local out = {}
		for _, t in list or {} do
			table.insert(out, session(t))
		end
		shelf:set(out)
	end, function() end)
end
ui.effect(function()
	if shared.details.archived:get() then
		read_shelf()
	end
end)

-- ── Choosing ───────────────────────────────────────────────────────────

local selected = shared.selected
ui.effect(function()
	local t = store.thread:get()
	local id = if t and nav.route:get() == "thread" then t.id else nil
	if selected:peek() ~= id then
		selected:set(id)
	end
end)
ui.effect(function()
	local id = selected:get()
	local t = store.thread:peek()
	if id and (not t or t.id ~= id or nav.route:peek() ~= "thread") then
		nav.go("thread")
		store.open(id)
	end
end)

local function thread_of(id: string): Thread?
	for _, t in store.threads:peek() do
		if t.id == id then
			return t
		end
	end
	return nil
end

local function project_of(id: string): any?
	for _, p in store.projects:peek() do
		if p.id == id then
			return p
		end
	end
	return nil
end

-- ── Places ─────────────────────────────────────────────────────────────

-- `id` moved to `where` (a section's id, "pinned"), or back to its project.
local function put(id: string, where: string?)
	local p = table.clone(shared.places:peek())
	local pinned = {}
	for _, x in p.pinned or {} do
		if x ~= id then
			table.insert(pinned, x)
		end
	end
	local sections = {}
	for _, s in p.sections or {} do
		local list = {}
		for _, x in s.sessions do
			if x ~= id then
				table.insert(list, x)
			end
		end
		if s.id == where then
			table.insert(list, id)
		end
		table.insert(sections, { id = s.id, name = s.name, sessions = list })
	end
	if where == "pinned" then
		table.insert(pinned, 1, id)
	end
	shared.places:set({ pinned = pinned, sections = sections })
end

function M.pin(id: string)
	local p = shared.places:peek()
	put(id, if table.find(p.pinned or {}, id) then nil else "pinned")
end

-- ── Acting on a thread ─────────────────────────────────────────────────

local renaming_row = ui.state("kx.sb.renaming_row", nil :: string?)
local deleting = ui.state("kx.sb.deleting", nil :: string?)
local delete_open = ui.state("kx.sb.delete_open", false)

function M.archive(id: string)
	if not fixture then
		ui.invoke("kx.archive", id)
	end
	put(id, nil)
	if selected:peek() == id then
		nav.go("home")
		store.home()
	end
	kx.toast { title = "Archived", body = "Its work and history are kept. Show archived lists it." }
	ui.after(400, read_shelf)
end

function M.rename(id: string)
	renaming_row:set(id)
end

local function copy(text: string?, what: string)
	if text and text ~= "" then
		ui.clipboard.write(text)
		kx.toast { title = `Copied {what}` }
	end
end

local function menu(id: string): { any }?
	local t = thread_of(id)
	if not t then
		return nil
	end
	local p = shared.places:peek()
	local is_pinned = table.find(p.pinned or {}, id) ~= nil
	local into: { any } = { { heading = "Sections" } }
	for _, s in p.sections or {} do
		local here = table.find(s.sessions, id) ~= nil
		table.insert(into, {
			label = s.name,
			icon = "folder",
			checked = function(): boolean
				return here
			end,
			on_select = function()
				put(id, if here then nil else s.id)
			end,
		})
	end
	table.insert(into, { separator = true })
	table.insert(into, {
		label = "New section",
		icon = "plus",
		on_select = function()
			local sid = `section-{os.time()}-{math.random(1, 1e6)}`
			local now = table.clone(shared.places:peek())
			local sections = table.clone(now.sections or {})
			table.insert(sections, { id = sid, name = "New section", sessions = {} })
			now.sections = sections
			shared.places:set(now)
			put(id, sid)
		end,
	})
	local project = project_of(t.project_id)
	local copies: { any } = {}
	if project and project.path then
		table.insert(copies, {
			label = "Path",
			icon = "folder",
			on_select = function()
				copy(project.path, "the path")
			end,
		})
	end
	table.insert(copies, {
		label = "Thread id",
		icon = "tag",
		on_select = function()
			copy(id, "the thread's id")
		end,
	})
	return {
		{ label = "Rename", icon = "pen", on_select = function()
			M.rename(id)
		end },
		{ label = if is_pinned then "Unpin" else "Pin", icon = "pin", on_select = function()
			M.pin(id)
		end },
		{ label = "Copy", icon = "copy", submenu = copies },
		{ label = "Move to", icon = "folder-with-files", submenu = into },
		{ separator = true },
		{ label = "Archive", icon = "archive-minimalistic", on_select = function()
			M.archive(id)
		end },
		{ label = "Delete", icon = "trash-bin-minimalistic", danger = true, on_select = function()
			deleting:set(id)
			delete_open:set(true)
		end },
	}
end
M.session_menu = menu

local removing = ui.state("kx.sb.removing", nil :: string?)
local remove_open = ui.state("kx.sb.remove_open", false)

local function project_menu(id: string): { any }?
	local p = project_of(id)
	if not p then
		return nil
	end
	return {
		{ label = "New thread here", icon = "pen-new-square", on_select = function()
			require("@screens/ray").project:set(id)
			nav.go("home")
			store.home()
			nav.focus_ray()
		end },
		{ label = "Show in folder", icon = "folder", on_select = function()
			if p.path then
				pcall(ui.invoke, "open_url", p.path)
			end
		end },
		{ label = "Copy path", icon = "copy", on_select = function()
			copy(p.path, "the path")
		end },
		{ separator = true },
		{ label = "Remove from Keplorayx", icon = "close", danger = true, on_select = function()
			removing:set(id)
			remove_open:set(true)
		end },
	}
end

local function dialogs(): Node
	return ui.column {
		ds.Dialog {
			open = delete_open,
			tone = "danger",
			title = function(): string
				local id = deleting:get()
				local t = if id then thread_of(id) else nil
				return if t then `Delete “{t.title}”?` else "Delete this thread?"
			end,
			body = "Its transcript goes for good, and a worktree made for it with it. Archive keeps them instead.",
			confirm = {
				label = "Delete",
				on_click = function()
					local id = deleting:peek()
					if id then
						put(id, nil)
						if not fixture then
							ui.invoke("kx.delete", id)
						end
						if selected:peek() == id then
							nav.go("home")
							store.home()
						end
					end
				end,
			},
		},
		ds.Dialog {
			open = remove_open,
			tone = "danger",
			title = function(): string
				local id = removing:get()
				local p = if id then project_of(id) else nil
				return if p then `Remove {p.name} from Keplorayx?` else "Remove this project?"
			end,
			body = "Its folder and its threads' files stay where they are; it leaves the sidebar. Add it again any time.",
			confirm = {
				label = "Remove",
				on_click = function()
					local id = removing:peek()
					if id and not fixture then
						ui.invoke("kx.remove_project", id)
					end
				end,
			},
		},
	}
end

-- ── Jump keys ──────────────────────────────────────────────────────────

-- The rows in order, as the list shows them (for Ctrl+1–9).
M.order = ui.state("kx.sb.order", {} :: { string })
-- Ctrl held a moment: each row's number shows.
M.jump_hints = ui.state("kx.sb.jump_hints", false)
local held = ui.modifiers
ui.effect(function()
	local on = held.control:get() and not held.alt:get() and not held.shift:get()
	if not on then
		M.jump_hints:set(false)
		return
	end
	ui.after(400, function()
		if held.control:peek() then
			M.jump_hints:set(true)
		end
	end)
end)
function M.jump(n: number)
	local id = M.order:peek()[n]
	if id then
		selected:set(id)
	end
end

-- ── The project filter (Show › Project filter) ─────────────────────────

local function project_filter(): Node
	return ui.show(
		shared.details.project_filter,
		ui.row {
			w_full = true,
			px = "$space.2",
			pb = "$space.1",
			ds.Select {
				value = ui.computed(function(): string
					return shared.project:get() or "::all"
				end),
				label = "Project",
				size = "sm",
				w_full = true,
				options = function(): { any }
					local out: { any } = { { value = "::all", label = "All projects", icon = "folder-with-files" } }
					for _, p in store.projects:get() do
						table.insert(out, { value = p.id, label = p.name })
					end
					return out
				end,
				on_change = function(v: string)
					shared.project:set(if v == "::all" then nil else v)
				end,
			},
		}
	)
end

-- ── The sidebar ────────────────────────────────────────────────────────

local profile_name = ui.persist("kx.profile.name", "")
local pages = ui.state("sidebar.pages", {} :: { any })

local function bar(island: boolean): Node
	return Sidebar {
		compact = shared.compact,
		organize = shared.organize :: any,
		organize_order = shared.organize_order,
		sort = shared.sort :: any,
		details = shared.details :: any,
		places = shared.places,
		width = shared.width,
		collapsed = nav.sidebar_collapsed,
		island = island,
		selected = selected,
		projects = projects,
		user = function()
			local n = profile_name:get()
			return { id = "you", name = if n ~= "" then n else "You" }
		end,
		-- Room for the window's title bar over its top.
		top = 38,
		pages = pages,
		on_page = function(p: any)
			nav.go(p.id)
		end,
		on_new = function(project: string?)
			if project then
				require("@screens/ray").project:set(project)
			end
			nav.go("home")
			store.home()
			nav.focus_ray()
		end,
		on_search = function()
			require("@screens/palette").toggle()
		end,
		on_settings = function()
			nav.go(if nav.route:peek() == "settings" then "home" else "settings")
		end,
		settings_open = function(): boolean
			return nav.route:get() == "settings"
		end,
		on_user = function()
			nav.go("settings")
		end,
		menu = menu,
		project_menu = project_menu,
		on_archive = M.archive,
		archived = shelf,
		on_unarchive = function(id: string)
			if not fixture then
				ui.invoke("kx.unarchive", id)
			end
			ui.after(400, read_shelf)
		end,
		on_rename = function(id: string, title: string)
			if not fixture then
				ui.invoke("kx.rename", { id = id, title = string.sub(title, 1, 200) })
			end
		end,
		renaming_session = if island then nil else renaming_row,
		narrowed = function(): boolean
			return shared.details.project_filter:get() and shared.project:get() ~= nil
		end,
		notice = if island then nil else project_filter(),
		offset = if island then nil else shared.offset,
		folded = if island then nil else shared.folded,
		expanded = if island then nil else shared.expanded,
		order = if island then nil else M.order,
		jump_hints = if island then nil else function(): boolean
			return M.jump_hints:get()
		end,
		-- Constellation and Team: quiet icons at its foot, beside Settings.
		tools = if island then nil else {
			{
				icon = "constellation",
				label = "Constellation",
				on = function(): boolean
					return nav.route:get() == "constellation"
				end,
				on_click = function()
					nav.go(if nav.route:peek() == "constellation" then "home" else "constellation")
				end,
			},
			{
				icon = "users-group",
				label = "Team",
				on = function(): boolean
					return nav.route:get() == "team"
				end,
				on_click = function()
					nav.go(if nav.route:peek() == "team" then "home" else "team")
				end,
			},
		},
	}
end

function M.view(): Node
	return ui.div {

		h_full = true,
		shrink = 0,
		relative = true,
		bar(false),
		dialogs(),
	}
end

-- Folded away (Ctrl+B): its rail, an island at the window's bottom left.
function M.rail(): Node
	return bar(true)
end

return M
