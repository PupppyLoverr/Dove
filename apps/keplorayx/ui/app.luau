--!strict
-- Keplorayx's main window: the sidebar, and one of Home (the Ray bar), a
-- thread, the Inbox or Settings beside it. Everything is Orbit (ui/kx);
-- the engine reaches it through kx/store.

ui.theme.strict("warn")
local kx = require("@kx")
kx.setup()

local store = require("@kx/store")
local nav = require("@screens/nav")
local sidebar = require("@screens/sidebar")
local home = require("@screens/home")
local thread = require("@screens/thread")
local inbox = require("@screens/inbox")
local settings = require("@screens/settings")
local ray = require("@screens/ray")
local constellation = require("@screens/constellation")

-- Development: a fixture's own starting route and theme (tools/shot.ps1).
if store.fixture ~= "" then
	local route = store.route or (if store.thread:peek() then "thread" else "home")
	nav.route:set(route)
end
local dev_theme = (ui.shared("dev.theme", "") :: any):peek() :: string
if dev_theme ~= "" then
	ui.theme.set(dev_theme)
end

-- The window's commands: in the menu, and their keys wherever focus is.
ui.command {
	id = "new-thread",
	title = "New thread",
	key = "ctrl+n",
	run = function()
		nav.go("home")
		store.home()
		nav.focus_ray()
	end,
}
ui.command {
	id = "search",
	title = "Search and commands",
	key = "ctrl+k",
	run = function()
		require("@screens/palette").toggle()
	end,
}
ui.command {
	id = "settings",
	title = "Settings",
	key = "ctrl+,",
	run = function()
		nav.go("settings")
	end,
}
ui.command {
	id = "changes",
	title = "Changes",
	key = "ctrl+j",
	run = function()
		require("@screens/panel").toggle("changes")
	end,
}
ui.command {
	id = "terminal",
	title = "Terminal",
	key = "ctrl+`",
	run = function()
		require("@screens/panel").toggle("terminal")
	end,
}
ui.command {
	id = "panel",
	title = "Show or hide the panel",
	key = "ctrl+\\",
	run = function()
		require("@screens/panel").toggle_panel()
	end,
}
ui.command {
	id = "back",
	title = "Back",
	key = "alt+left",
	run = nav.back,
}
ui.command {
	id = "forward",
	title = "Forward",
	key = "alt+right",
	run = nav.forward,
}
ui.command {
	id = "files",
	title = "Files",
	key = "ctrl+p",
	run = function()
		require("@screens/panel").toggle("files")
	end,
}
ui.command {
	id = "browser",
	title = "Browser",
	key = "ctrl+shift+b",
	run = function()
		require("@screens/panel").toggle("browser")
	end,
}
ui.command {
	id = "sidebar",
	title = "Show or hide the sidebar",
	key = "ctrl+b",
	run = function()
		nav.sidebar_open:set(not nav.sidebar_open:peek())
	end,
}
ui.command {
	id = "team",
	title = "Team",
	key = "ctrl+shift+t",
	run = function()
		nav.go("team")
	end,
}
ui.command {
	id = "constellation",
	title = "Constellation",
	key = "ctrl+shift+o",
	run = function()
		nav.go(if nav.route:peek() == "constellation" then "home" else "constellation")
	end,
}
ui.command {
	id = "zoom-out",
	title = "Zoom out to every thread",
	key = "ctrl+-",
	run = function()
		nav.go("constellation")
	end,
}
ui.command {
	id = "inbox",
	title = "Inbox",
	key = "ctrl+i",
	run = function()
		nav.go("inbox")
	end,
}

-- The notch asked for a thread ("<id>#<n>", n so the same thread twice is
-- two asks): open it.
local ok_focus, notch_focus = pcall(ui.shared, "notch.focus", "")
if ok_focus and store.fixture == "" then
	ui.effect(function()
		local v = (notch_focus :: any):get() :: string
		local id = string.match(v, "^(.-)#")
		if id and id ~= "" then
			nav.go("thread")
			store.open(id)
		end
	end)
end

-- The tray and the keys from any app (src/platform.rs) ask through
-- `kx.shell` ("<word>#<n>"): open, ask (a new thread, the composer
-- focused), inbox, settings; allow and deny answer the oldest request
-- waiting on you, wherever it is.
local ok_shell, shell = pcall(ui.shared, "kx.shell", "")
if ok_shell and store.fixture == "" then
	ui.effect(function()
		local v = (shell :: any):get() :: string
		local word = string.match(v, "^(.-)#")
		if word == "ask" then
			nav.go("home")
			store.home()
			nav.focus_ray()
		elseif word == "inbox" then
			nav.go("inbox")
		elseif word == "settings" then
			nav.go("settings")
		elseif word == "allow" or word == "deny" then
			local waiting = (ui.shared("kx.attention", {}) :: any):peek()
			local first = waiting and waiting[1]
			if first then
				ui.invoke("kx.decide_for", {
					thread_id = first.thread_id,
					request_id = first.request_id,
					decision = if word == "allow" then "allow_once" else "deny",
				})
			end
		end
	end)
end

-- A word from the system when a thread needs you, or finishes while the
-- window is away (Settings › Notifications says which).
local notify_needs = ui.persist("kx.notify.needs", true)
local notify_done = ui.persist("kx.notify.done", true)
local seen_status: { [string]: string } = {}
-- Whether this window is the one you're in (src/main.rs publishes it).
local function window_active(): boolean
	local ok, v = pcall(function(): any
		return (ui :: any).window.active:peek()
	end)
	return ok and v == true
end
if store.fixture == "" then
	ui.effect(function()
		local threads = store.threads:get()
		local first = next(seen_status) == nil
		for _, t in threads do
			local was = seen_status[t.id]
			seen_status[t.id] = t.status
			if not first and was ~= t.status and not window_active() then
				if t.status == "needs_you" and notify_needs:peek() then
					pcall(ui.notify, { title = `{t.harness} needs you`, body = t.title })
				elseif was == "running" and t.status == "done" and notify_done:peek() then
					pcall(ui.notify, { title = `{t.harness} finished`, body = t.title })
				end
			end
		end
	end)
	-- The taskbar's badge: how many wait on you.
	ui.effect(function()
		local n = 0
		for _, t in store.threads:get() do
			if t.status == "needs_you" then
				n += 1
			end
		end
		pcall(ui.badge, if n > 0 then tostring(n) else nil)
	end)
end

-- Y and N answer what the open thread waits on, when focus isn't in a field.
local function answer(e: any): boolean?
	local a = store.approval:peek()
	if not a or nav.route:peek() ~= "thread" or e.editing or e.ctrl or e.meta or e.alt then
		return nil
	end
	if e.key == "y" then
		store.decide(a.request_id, "allow_once")
		return true
	elseif e.key == "n" then
		store.decide(a.request_id, "deny")
		return true
	end
	return nil
end

local pages = {
	home = ui.column { grow = 1, min_w = 0, h_full = true, home.view() },
	thread = ui.column { grow = 1, min_w = 0, h_full = true, thread.view() },
	inbox = ui.column { grow = 1, min_w = 0, h_full = true, inbox.view() },
	settings = ui.column { grow = 1, min_w = 0, h_full = true, settings.view() },
	constellation = ui.column { grow = 1, min_w = 0, h_full = true, constellation.view() },
	team = ui.column { grow = 1, min_w = 0, h_full = true, require("@screens/team").view() },
}
local panel = require("@screens/panel")

-- A folder dropped on the window becomes a project (a file, its folder).
local dragging = ui.state("app.dragging", false)
local function on_drop(files: { any })
	dragging:set(false)
	for _, f in files do
		local path: string = f.path
		if f.type ~= nil and f.type ~= "" then
			path = string.match(path, "^(.*)[/\\][^/\\]+$") or path
		end
		store.add_project(path):after(function(id: any)
			if type(id) == "string" then
				ray.project:set(id)
			end
			kx.toast { title = `Added {string.match(path, "([^/\\]+)$") or path}` }
		end, function()
			kx.toast { title = "That couldn't be added", body = path, kind = "error" }
		end)
	end
end

local drop_veil = ui.column {
	absolute = true,
	top = 0,
	left = 0,
	right = 0,
	bottom = 0,
	items = "center",
	justify = "center",
	gap = "$space.2",
	pointer_events = false,
	bg = "$kx.veil",
	border = 2,
	border_dash = { 6, 6 },
	border_color = "$accent",
	rounded = "$radius.xl",
	ui.column { color = "$accent", kx.Icon { name = "folder-add", size = 32 } },
	kx.Label { "Drop a folder to work in it", literal = true },
}

-- The title bar: Zeron's (ds/title_bar), one for the window, hung over its
-- top. At its left the sidebar's toggle, back and forward; at its right the
-- panel's toggle (the corner card: a card in a frame's top right, as
-- Zeron draws it). Over Home's sky, an island of glass gathers behind them.
local TitleBar = require("@ds/title_bar")

local function corner_card(size: number): Node
	local s = size / 24
	local function raw(v: number): any
		return ui.raw(v, "the glyph's own geometry, scaled from its 24-unit source")
	end
	return ui.div {
		w = size,
		h = size,
		shrink = 0,
		ui.div {
			absolute = true,
			left = raw(2.2 * s),
			top = raw(3.2 * s),
			w = raw(19.6 * s),
			h = raw(17.6 * s),
			rounded = raw(4.8 * s),
			border = 1.5,
			border_color = "$text.secondary",
		},
		ui.div {
			absolute = true,
			right = raw(6.5 * s),
			top = raw(7.5 * s),
			w = raw(5.5 * s),
			h = raw(4.5 * s),
			rounded = raw(0.9 * s),
			bg = function()
				return if panel.open:get() then "$text.secondary" else "#00000000"
			end,
			border = 1.2,
			border_color = "$text.secondary",
		},
	}
end

local function over_sky(): boolean
	return nav.route:get() == "home" and require("@screens/home").sky:get() ~= "none"
end

local title_bar = ui.div {
	absolute = true,
	top = 0,
	left = 0,
	right = 0,
	z = 20,
	TitleBar {
		collapsed = nav.sidebar_collapsed,
		can_back = nav.can_back,
		can_forward = nav.can_forward,
		on_back = nav.back,
		on_forward = nav.forward,
		island = function(): boolean
			return nav.sidebar_collapsed:get() and over_sky()
		end,
		actions_island = over_sky,
		actions = ui.show(function()
			return nav.route:get() ~= "settings"
		end, TitleBar.control(
			"Show or hide the panel (Ctrl+\\)",
			ui.column { color = "$text.secondary", corner_card(18) },
			nil,
			panel.toggle_panel
		)),
	},
}

-- What stands at the window's left: the sidebar, or (in Settings) Settings'
-- own pages, as Zeron's settings take the sidebar's place.
local left = ui.switch(function(): string
	return if nav.route:get() == "settings" then "settings" else "sidebar"
end, {
	sidebar = sidebar.view(),
	settings = settings.nav(),
})

return ui.row {
	w_full = true,
	h_full = true,
	relative = true,
	theme = kx.accent.scope(),
	bg = kx.material.window_bg(),
	on_key = answer,
	on_file_drop = on_drop,
	on_file_hover = function(over: boolean)
		dragging:set(over)
	end,
	left,
	-- The page: a card set into the window (Zeron's, as Dove's was), lighter
	-- than the glass round it, its corner the window's less the gap.
	ui.column {
		grow = 1,
		-- From nothing, it takes the room the sidebar and panel leave: its
		-- content's width never pushes the window wider.
		basis = 0,
		min_w = 0,
		relative = true,
		overflow = "hidden",
		my = "$zeron.inset.gap",
		mr = "$zeron.inset.gap",
		rounded = "$zeron.inset.radius",
		bg = "$zeron.inset.bg",
		border = 1,
		border_color = "$zeron.inset.edge",
		ui.switch(nav.route, pages, pages.home),
	},
	-- The panel, beside whatever page shows (Files, the Browser, …).
	ui.presence(function()
		return if panel.open:get() and nav.route:get() ~= "settings" then "open" else nil
	end, function()
		return panel.view()
	end, {
		h_full = true,
		enter = { opacity = { 0, 1 }, x = { 24, 0 }, duration = 220, ease = kx.motion.ENTER },
		exit = { opacity = { 1, 0 }, x = { 0, 16 }, duration = 120 },
	}),
	ui.show(dragging, drop_veil),
	title_bar,
	kx.Toasts {},
	require("@screens/palette").view(),
}
