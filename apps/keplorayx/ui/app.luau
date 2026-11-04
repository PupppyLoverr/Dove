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
	run = nav.focus_ray,
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
	id = "inbox",
	title = "Inbox",
	key = "ctrl+i",
	run = function()
		nav.go("inbox")
	end,
}

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
}

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

-- Home's own title bar: nothing but room to drag and the caption buttons.
local home_bar = ui.row {
	absolute = true,
	top = 0,
	left = 0,
	right = 0,
	h = "$kx.titlebar",
	window_drag = true,
}

return ui.row {
	w_full = true,
	h_full = true,
	relative = true,
	bg = kx.material.window_bg(),
	on_key = answer,
	on_file_drop = on_drop,
	on_file_hover = function(over: boolean)
		dragging:set(over)
	end,
	sidebar.view(),
	ui.column {
		grow = 1,
		min_w = 0,
		h_full = true,
		relative = true,
		ui.show(function()
			return nav.route:get() == "home"
		end, home_bar),
		ui.switch(nav.route, pages, pages.home),
	},
	ui.show(dragging, drop_veil),
	kx.Toasts {},
}
