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

local projects = ui.computed(function(): { any }
	local by: { [string]: any } = {}
	local out: { any } = {}
	for _, p in store.projects:get() do
		local g = { id = p.id, name = p.name, sessions = {} }
		by[p.id] = g
		table.insert(out, g)
	end
	for _, t in store.threads:get() do
		if not pinned(t) then
			local g = by[t.project_id]
			if not g then
				g = { id = t.project_id, name = t.project, sessions = {} }
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
local PAGES = {
	{ plugin = "kx", id = "constellation", title = "Constellation", icon = "constellation" },
	{ plugin = "kx", id = "team", title = "Team", icon = "users-group" },
}
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

local function menu(id: string): { any }?
	local is_pinned = nav.pinned:peek()[id] == true
	return {
		{
			label = if is_pinned then "Unpin" else "Pin to the top",
			icon = "pin",
			on_click = function()
				nav.toggle_pin(id)
			end,
		},
		{
			label = "Open",
			icon = "arrow-up-right",
			on_click = function()
				selected:set(id)
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
	}
end

return M
