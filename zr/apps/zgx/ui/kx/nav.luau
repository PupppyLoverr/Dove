--!strict
-- The sidebar's tabs under New and Search, as the old Keplorayx had them
-- (apps/dove/src/sidebar.rs): Projects, Artifacts, Customize and More.
-- Each is a row the sidebar lists as it lists a plugin's page (under the
-- `kx` plugin name), so they follow the sidebar's own design.
--
--   nav.rows(filter_on)   Projects, Artifacts, Customize (before plugins' pages)
--   nav.more()            More, and under it, while it's open, what it holds
--   nav.open(page, toggle_filter)   what a click on one does; false for a
--                                   row that isn't one of these
--
-- Projects shows the project filter over the list (the old Projects menu
-- narrowed the list the same way). Artifacts and Kanban are pages
-- (ui/kx/pages). Customize is Settings › Appearance. More opens in place:
-- pressed, its rows show under it (Kanban, then Team, Agents and Archived,
-- which are settings' own pages here); pressed again, they fold away. (A
-- menu at the pointer was tried first, as the old app had: it never showed.)

local route = require("@app/screens/shell/route")

local M = {}

M.PLUGIN = "kx"

-- Whether More shows what it holds (kept across launches).
local more_open = ui.persist("kx_nav_more_open", false)

local function on_page(cur: any, id: string): boolean
	return cur.name == "plugin" and cur.plugin == M.PLUGIN and cur.page == id
end
local function on_settings(cur: any, page: string): boolean
	return cur.name == "settings" and cur.page == page
end

function M.rows(filter_on: () -> boolean): { any }
	local cur: any = route.current:get()
	return {
		{ plugin = M.PLUGIN, id = "projects", title = "Projects", icon = "folder-with-files", selected = filter_on() },
		{ plugin = M.PLUGIN, id = "artifacts", title = "Artifacts", icon = "layers", selected = on_page(cur, "artifacts") },
		{ plugin = M.PLUGIN, id = "customize", title = "Customize", icon = "tuning", selected = on_settings(cur, "appearance") },
	}
end

function M.more(): { any }
	local cur: any = route.current:get()
	local open = more_open:get()
	local kanban = on_page(cur, "kanban")
	local out: { any } = {
		-- (Folded with Kanban showing, it's lit: that's where you are.)
		{ plugin = M.PLUGIN, id = "more", title = if open then "Less" else "More", icon = "more-horizontal", selected = kanban and not open },
	}
	if open then
		table.insert(out, { plugin = M.PLUGIN, id = "kanban", title = "Kanban", icon = "widget", selected = kanban })
		table.insert(out, { plugin = M.PLUGIN, id = "team", title = "Team", icon = "users-group", selected = on_settings(cur, "members") })
		table.insert(out, { plugin = M.PLUGIN, id = "agents", title = "Agents", icon = "bot", selected = on_settings(cur, "providers") })
		table.insert(out, { plugin = M.PLUGIN, id = "archived", title = "Archived", icon = "archive-minimalistic", selected = on_settings(cur, "archived") })
	end
	return out
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
		more_open:set(not more_open:peek())
	elseif page.id == "kanban" then
		route.plugin_page(M.PLUGIN, "kanban")
	elseif page.id == "team" then
		route.settings("members")
	elseif page.id == "agents" then
		route.settings("providers")
	elseif page.id == "archived" then
		route.settings("archived")
	else
		return false
	end
	return true
end

return M
