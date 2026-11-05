--!strict
-- The sidebar: Zeron's (ds/sidebar, vendored; MIT, see
-- THIRD_PARTY_NOTICES.md), worn as Dove's was. New thread on top as a
-- lifted button with its key, then Search and the way in (Inbox,
-- Constellation, Team); then the threads on two lines, as Dove listed
-- them: its agent's mark, its title, and "project · branch" under it, its
-- corner saying what it's doing (the working ring, a question or an
-- approval waiting on you, a failure, work done you haven't seen) or how
-- long ago it moved. Pinned threads lead. Its view menu ("…") groups by
-- project, folds to one line a thread, and the rest of Zeron's choices; it
-- folds away whole with Ctrl+B. You and Settings at the foot.

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

local function session(t: Thread): any
	return {
		id = t.id,
		title = t.title,
		status = kind(t),
		-- (ms in the store; the sidebar counts in seconds.)
		time = math.floor(t.updated / 1000),
		agent = MARK[t.harness_id],
		branch = if t.branch and t.branch ~= "" then t.branch else nil,
		created = math.floor(t.updated / 1000),
	}
end

-- Pinned: the store's flag (Dove's), or yours on this device.
local function pinned(t: Thread): boolean
	return t.pinned == true or nav.pinned:get()[t.id] == true
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

local projects = ui.computed(function(): { any }
	local by: { [string]: any } = {}
	local out: { any } = {}
	for _, p in store.projects:get() do
		local g = { id = p.id, name = p.name, hue = hue_of(p.name), sessions = {} }
		by[p.id] = g
		table.insert(out, g)
	end
	for _, t in store.threads:get() do
		if not pinned(t) then
			local g = by[t.project_id]
			if not g then
				g = { id = t.project_id, name = t.project, hue = hue_of(t.project), sessions = {} }
				by[t.project_id] = g
				table.insert(out, g)
			end
			table.insert(g.sessions, session(t))
		end
	end
	return out
end)

local pinned_list = ui.computed(function(): { any }
	local out: { any } = {}
	for _, t in store.threads:get() do
		if pinned(t) then
			table.insert(out, session(t))
		end
	end
	return out
end)

-- The way in, as rows under Search.
-- (Plugins add theirs; Constellation and Team are the foot's tools.)
local PAGES = {}
local pages = ui.state("sidebar.pages", PAGES)

local profile_name = ui.persist("kx.profile.name", "")

-- Which thread shows: the open one's id, and choosing one opens it.
local selected = ui.state("sidebar.selected", nil :: string?)
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

-- Pinned in the store (every window, and the launcher, agree); an old
-- local pin is cleared as it's flipped.
local function toggle_pin(id: string)
	local t = thread_of(id)
	local on = not (t and pinned(t))
	if nav.pinned:peek()[id] then
		nav.toggle_pin(id)
	end
	if store.fixture == "" then
		ui.invoke("kx.pin", { id = id, on = on })
	end
end

local function archive(id: string)
	if store.fixture == "" then
		ui.invoke("kx.archive", id)
	end
	if selected:peek() == id then
		nav.go("home")
		store.home()
	end
	kx.toast { title = "Archived", body = "Its work and history are kept." }
end

local function menu(id: string): { any }?
	local t = thread_of(id)
	local is_pinned = t ~= nil and pinned(t)
	return {
		{
			label = if is_pinned then "Unpin" else "Pin to the top",
			icon = "pin",
			on_click = function()
				toggle_pin(id)
			end,
		},
		{
			label = "Open",
			icon = "arrow-up-right",
			on_click = function()
				selected:set(id)
			end,
		},
		{ separator = true },
		{
			label = "Archive",
			icon = "archive-minimalistic",
			on_click = function()
				archive(id)
			end,
		},
	}
end

function M.view(): Node
	return Sidebar {
		compact = nav.sidebar_compact,
		collapsed = nav.sidebar_collapsed,
		width = nav.sidebar_width,
		organize = nav.sidebar_organize,
		details = nav.sidebar_details :: any,
		folded = nav.folded,
		selected = selected,
		pinned = pinned_list,
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
		on_archive = archive,
		on_rename = function(id: string, title: string)
			if store.fixture == "" then
				ui.invoke("kx.rename", { id = id, title = title })
			end
		end,
		-- Constellation and Team: quiet icons at its foot, beside Settings,
		-- so its top is only New thread and Search.
		tools = {
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

return M
