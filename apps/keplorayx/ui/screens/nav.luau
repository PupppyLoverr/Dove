--!strict
-- Where the window is: one route, kept across reloads (`home`, `thread`,
-- `inbox`, `settings`), and the Ray bar's focus, which any screen can ask
-- for (Ctrl+K, the sidebar's Search).

local M = {}

export type Route = "home" | "thread" | "inbox" | "settings"

M.route = ui.state("nav.route", "home" :: Route)

-- Bumped to ask the Ray bar for focus; it listens and takes it.
M.ray_focus = ui.state("nav.ray_focus", 0)

-- The Ray bar's mode, set by its leading character or a shortcut:
-- `ask` (a prompt), `command` (>), `mention` (@), `slash` (/).
M.ray_mode = ui.state("nav.ray_mode", "ask")

-- The room a title bar's right end leaves for the window's caption buttons
-- (Windows: minimise, maximise, close), when it is the one under them.
function M.caption_inset(): any
	local area, buttons = ui.window.titlebar_area:get(), ui.window.controls:get()
	local inset = if area and buttons and buttons.x >= area.x + area.width then buttons.width + 12 else 12
	return ui.raw(inset, "clears the window's caption buttons, which the system places")
end

function M.go(route: Route)
	M.route:set(route)
end

function M.focus_ray()
	if M.route:peek() ~= "thread" then
		M.route:set("home")
	end
	M.ray_focus:set(M.ray_focus:peek() + 1)
end

return M
