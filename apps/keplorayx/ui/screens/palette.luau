--!strict
-- Ctrl+K: Zeron's command palette (ds/command_palette, vendored; MIT, see
-- THIRD_PARTY_NOTICES.md), holding Keplorayx's things. One field finds a
-- thread, an action, a place or a setting, and does it; its scopes (All,
-- Threads, Commands, Settings) narrow it; pages open from rows (Theme ›,
-- Sky ›, Agent ›), each a chip in the field.

local CommandPalette = require("@ds/command_palette")
local kx = require("@kx")
local store = require("@kx/store")
local nav = require("@screens/nav")

local M = {}

M.open = ui.state("palette.open", false)

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

local function kind(t: store.Thread): string?
	if t.status == "needs_you" then
		return "approval"
	elseif t.status == "running" then
		return "working"
	elseif t.unread and t.status == "done" then
		return "done"
	end
	return nil
end

local function panel(tab: string): () -> ()
	return function()
		require("@screens/panel").show(tab)
	end
end

local function go(route: any): () -> ()
	return function()
		nav.go(route)
	end
end

local function threads(): { any }
	local out: { any } = {}
	for _, t in store.threads:get() do
		table.insert(out, {
			id = `thread:{t.id}`,
			label = t.title,
			section = "Threads",
			mark = MARK[t.harness_id],
			session = {
				status = kind(t),
				time = math.floor(t.updated / 1000),
				agent = MARK[t.harness_id],
				project = t.project,
				branch = if t.branch and t.branch ~= "" then t.branch else nil,
			},
			verb = "Open",
			run = function()
				nav.go("thread")
				store.open(t.id)
			end,
		})
	end
	return out
end

local function theme_page(): any
	local options = {
		{ id = "orbit-night", label = "Night", icon = "moon" },
		{ id = "orbit-dawn", label = "Dawn", icon = "sun" },
	}
	local items: { any } = {}
	for _, o in options do
		table.insert(items, {
			id = `theme:{o.id}`,
			label = o.label,
			section = "Theme",
			icon = o.icon,
			checked = function()
				return ui.theme.current:get() == o.id
			end,
			run = function()
				ui.persist("settings.scheme", "system"):set(o.id)
				ui.theme.set(o.id)
			end,
		})
	end
	return { id = "theme", title = "Theme", items = items }
end

local function sky_page(): any
	local home = require("@screens/home")
	local items: { any } = {}
	for _, s in home.SKIES do
		table.insert(items, {
			id = `sky:{s.value}`,
			label = s.label,
			section = "Sky",
			icon = "gallery",
			checked = function()
				return home.sky:get() == s.value
			end,
			run = function()
				home.sky:set(s.value)
			end,
		})
	end
	return { id = "sky", title = "Sky", items = items }
end

local function commands(): { any }
	return {
		{ id = "new", label = "New thread", section = "Actions", icon = "pen-new-square", shortcut = "ctrl+n", pinned = true, run = function()
			nav.go("home")
			store.home()
			nav.focus_ray()
		end },
		{ id = "changes", label = "Review the changes", section = "Actions", icon = "file-diff", shortcut = "ctrl+j", run = panel("changes") },
		{ id = "files", label = "Browse the project's files", section = "Actions", icon = "folder-with-files", shortcut = "ctrl+p", run = panel("files") },
		{ id = "browser", label = "Open the browser", section = "Actions", icon = "global", shortcut = "ctrl+shift+b", run = panel("browser") },
		{ id = "terminal", label = "Open a terminal", section = "Actions", icon = "terminal", run = panel("terminal") },
		{ id = "history", label = "Show the project's history", section = "Actions", icon = "git-branch", run = panel("history") },
		{ id = "pulls", label = "Show pull requests", section = "Actions", icon = "pull-request", run = panel("pulls") },
		{ id = "add-project", label = "Add a project folder…", section = "Actions", icon = "folder", run = function()
			require("@screens/ray").pick_folder()
		end },
		{ id = "sidebar", label = "Show or hide the sidebar", section = "Actions", icon = "sidebar", shortcut = "ctrl+b", run = function()
			nav.sidebar_collapsed:set(not nav.sidebar_collapsed:peek())
		end },
		{ id = "inbox", label = "Inbox", section = "Go to", icon = "inbox", shortcut = "ctrl+i", run = go("inbox") },
		{ id = "team", label = "Team", section = "Go to", icon = "users-group", run = go("team") },
		{ id = "constellation", label = "Constellation", section = "Go to", icon = "constellation", run = go("constellation") },
		{ id = "settings", label = "Settings", section = "Go to", icon = "settings", shortcut = "ctrl+,", run = go("settings") },
		{ id = "theme", label = "Theme", section = "Settings", icon = "palette", keywords = "dark light night dawn appearance", page = theme_page },
		{ id = "sky", label = "Sky behind a new thread", section = "Settings", icon = "gallery", keywords = "wallpaper background art", page = sky_page },
		{ id = "motion", label = "Reduce motion", section = "Settings", icon = "waveform", toggle = kx.motion.reduce },
		{ id = "glass", label = "Frosted glass", section = "Settings", icon = "layers", keywords = "transparency blur", toggle = kx.material.frosted },
		{ id = "compact", label = "One line a thread", section = "Settings", icon = "list", keywords = "sidebar density compact", toggle = nav.sidebar_compact },
	}
end

local root = {
	id = "root",
	title = "Keplorayx",
	placeholder = "Find a thread, run a command…",
	scopes = true,
	items = function(): { any }
		local out = commands()
		for _, t in threads() do
			table.insert(out, t)
		end
		return out
	end,
}

function M.view(): Node
	return CommandPalette { open = M.open, root = root :: any }
end

function M.toggle()
	M.open:set(not M.open:peek())
end

return M
