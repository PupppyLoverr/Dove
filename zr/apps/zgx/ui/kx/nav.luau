--!strict
-- The sidebar's tabs under New and Search, as the old Keplorayx had them
-- (apps/dove/src/sidebar.rs): Projects, Artifacts, Customize and More.
-- Each is a row the sidebar lists as it lists a plugin's page (under the
-- `kx` plugin name), so they follow the sidebar's own design.
--
--   nav.rows(filter_on)   Projects, Artifacts, Customize (before plugins' pages)
--   nav.more()            More (after them)
--   nav.press(page, e)    where a row was pressed (More opens its menu there)
--   nav.open(page, toggle_filter)   what a click on one does; false for a
--                                   row that isn't one of these
--
-- Projects shows the project filter over the list (the old Projects menu
-- narrowed the list the same way). Artifacts and Kanban are pages
-- (ui/kx/pages). Customize is Settings › Appearance. More is a menu: Kanban,
-- then Team, Agents and Archived, which are settings' own pages here.

local route = require("@app/screens/shell/route")
local MenuAt = require("@ds/menu_at")

local M = {}

M.PLUGIN = "kx"

-- Where the last row was pressed, in the window: More's menu opens there.
local pressed: { x: number, y: number }? = nil

local function on_page(cur: any, id: string): boolean
	return cur.name == "plugin" and cur.plugin == M.PLUGIN and cur.page == id
end

function M.rows(filter_on: () -> boolean): { any }
	local cur: any = route.current:get()
	return {
		{ plugin = M.PLUGIN, id = "projects", title = "Projects", icon = "folder-with-files", selected = filter_on() },
		{ plugin = M.PLUGIN, id = "artifacts", title = "Artifacts", icon = "layers", selected = on_page(cur, "artifacts") },
		{
			plugin = M.PLUGIN,
			id = "customize",
			title = "Customize",
			icon = "tuning",
			selected = cur.name == "settings" and cur.page == "appearance",
		},
	}
end

function M.more(): any
	local cur: any = route.current:get()
	return { plugin = M.PLUGIN, id = "more", title = "More", icon = "more-horizontal", selected = on_page(cur, "kanban") }
end

function M.press(page: any, e: any)
	if page.plugin == M.PLUGIN and e and type(e.x) == "number" and type(e.y) == "number" then
		pressed = { x = e.x, y = e.y }
	end
end

local function more_menu(): { any }
	return {
		{
			label = "Kanban",
			icon = "widget",
			on_select = function()
				route.plugin_page(M.PLUGIN, "kanban")
			end,
		},
		{
			label = "Team",
			icon = "users-group",
			on_select = function()
				route.settings("members")
			end,
		},
		{
			label = "Agents",
			icon = "bot",
			on_select = function()
				route.settings("providers")
			end,
		},
		{
			label = "Archived",
			icon = "archive-minimalistic",
			on_select = function()
				route.settings("archived")
			end,
		},
	}
end

function M.open(page: any, toggle_filter: () -> ()): boolean
	if page.plugin ~= M.PLUGIN then
		return false
	end
	if page.id == "projects" then
		toggle_filter()
	elseif page.id == "artifacts" then
		route.plugin_page(M.PLUGIN, "artifacts")
	elseif page.id == "customize" then
		route.settings("appearance")
	elseif page.id == "more" then
		-- (Its menu where it was pressed; from the keys, with no press to
		-- stand it at, its first page.)
		local at = pressed
		pressed = nil
		if at then
			MenuAt.show(at :: any, more_menu())
		else
			route.plugin_page(M.PLUGIN, "kanban")
		end
	else
		return false
	end
	return true
end

return M
