--!strict
-- The panel beside a thread or Home, as Zeron's side panel is (screens/
-- panel; MIT, see THIRD_PARTY_NOTICES.md): one view at a time under a row
-- of icon tabs. Changes (Ctrl+J), Files (Ctrl+P), the Browser
-- (Ctrl+Shift+B), the Terminal (Ctrl+`), History and Pull requests. A tab's
-- key again closes it; the panel's edge drags its width.

local kx = require("@kx")
local nav = require("@screens/nav")
local changes = require("@screens/changes")
local terminal = require("@screens/terminal")
local files = require("@screens/files")
local browser = require("@screens/browser")
local history = require("@screens/history")
local pulls = require("@screens/pulls")
local store = require("@kx/store")

local M = {}

-- Open or not lives with Changes (its refresh listens), the tab here.
M.open = changes.open
M.tab = if store.fixture ~= "" then ui.state("panel.tab", "changes") else ui.persist("panel.tab", "changes")
M.width = ui.persist("panel.width", 480)

-- Development: a fixture may ask for a tab.
local fx = (store :: any).fixture_data
if fx and fx.panel_tab then
	M.tab:set(fx.panel_tab)
	M.open:set(true)
end

function M.toggle(tab: string)
	if M.open:peek() and M.tab:peek() == tab then
		M.open:set(false)
	else
		M.tab:set(tab)
		M.open:set(true)
	end
end

function M.show(tab: string)
	M.tab:set(tab)
	M.open:set(true)
end

local TABS = {
	{ id = "changes", icon = "file-diff", label = "Changes", keys = "Ctrl+J" },
	{ id = "files", icon = "folder-with-files", label = "Files", keys = "Ctrl+P" },
	{ id = "browser", icon = "global", label = "Browser", keys = "Ctrl+Shift+B" },
	{ id = "terminal", icon = "terminal", label = "Terminal", keys = "Ctrl+`" },
	{ id = "history", icon = "git-branch", label = "History", keys = "" },
	{ id = "pulls", icon = "pull-request", label = "Pull requests", keys = "" },
}

local function tab_button(t: { id: string, icon: string, label: string, keys: string }): Node
	return ui.button {
		w = 28,
		h = 28,
		items = "center",
		justify = "center",
		rounded = "$radius.md",
		cursor = "pointer",
		label = t.label,
		tooltip = if t.keys ~= "" then `{t.label}  {t.keys}` else t.label,
		bg = function()
			return if M.tab:get() == t.id then "$selection.item" else "#00000000"
		end,
		color = function()
			return if M.tab:get() == t.id then "$text.primary" else "$text.tertiary"
		end,
		hover = { bg = "$hover", color = "$text.primary" },
		on_click = function()
			M.tab:set(t.id)
		end,
		kx.Icon { name = t.icon, size = 16 },
	}
end

local VIEWS: { [string]: () -> Node } = {
	changes = changes.list,
	files = files.view,
	browser = browser.view,
	terminal = terminal.view,
	history = history.view,
	pulls = pulls.view,
}

-- The edge between the page and the panel: drag to size it.
local function edge(): Node
	local from_x, from_w = 0, 0
	local dragging = ui.state("panel.dragging", false)
	local over = ui.state("panel.edge_over", false)
	return ui.row {
		absolute = true,
		left = -3,
		top = 0,
		bottom = 0,
		w = 6,
		z = 5,
		justify = "center",
		cursor = "col_resize",
		label = "Resize the panel",
		on_hover = function(inside: boolean)
			over:set(inside)
		end,
		on_press = function(e: any)
			from_x, from_w = e.x, M.width:peek()
			dragging:set(true)
		end,
		on_pointer_move = function(e: any)
			if dragging:peek() then
				M.width:set(math.clamp(from_w - (e.x - from_x), 340, 960))
			end
		end,
		on_release = function()
			dragging:set(false)
		end,
		ui.div {
			w = 1,
			h_full = true,
			pointer_events = "none",
			bg = function()
				return if over:get() or dragging:get() then "$border.strong" else "#00000000"
			end,
		},
	}
end

function M.view(): Node
	return ui.column {
		-- As wide as it was dragged, short of leaving the page less than a
		-- readable column beside the sidebar.
		w = function(): number
			local ok, size = pcall(function(): any
				return (ui :: any).window.size:get()
			end)
			local window = if ok and type(size) == "table" and size.width then size.width else 1240
			local sidebar = if nav.sidebar_collapsed:get() then 0 else nav.sidebar_width:get()
			-- (A page narrower than about 460 points stops reading well.)
			return math.max(320, math.min(M.width:get(), window - sidebar - 460))
		end,
		shrink = 0,
		relative = true,
		overflow = "hidden",
		-- A second card beside the page's, as Zeron sets the panel.
		my = "$zeron.inset.gap",
		mr = "$zeron.inset.gap",
		rounded = "$zeron.inset.radius",
		bg = "$zeron.inset.bg",
		border = 1,
		border_color = "$zeron.inset.edge",
		edge(),
		ui.row {
			h = "$kx.titlebar",
			w_full = true,
			pl = "$space.2",
			pr = nav.caption_inset,
			gap = "$kx.space.hair",
			items = "center",
			window_drag = true,
			ui.each(TABS, tab_button, { key = function(t)
				return t.id
			end, gap = "$kx.space.hair" }),
			ui.div { grow = 1 },
			ui.show(function()
				return M.tab:get() == "changes"
			end, changes.summary()),
		},
		-- Only the tab that shows is built (and reads what it shows).
		ui.presence(M.tab, function(tab: string): Node
			local build = VIEWS[tab] or VIEWS.changes
			return ui.column { grow = 1, min_h = 0, w_full = true, build() }
		end, { grow = 1, min_h = 0, w_full = true }),
	}
end

return M
