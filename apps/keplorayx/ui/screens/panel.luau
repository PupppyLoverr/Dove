--!strict
-- The right panel, as Zeron's is (screens/panel, ds/panel_home; MIT, see
-- THIRD_PARTY_NOTICES.md): a strip of the tabs you opened, each as wide as
-- its name with × under the pointer, and "+" for its home. The home says
-- where the thread works (its project, its branch and what it changed,
-- +N −M) and lists what can open here, each with the one key that opens
-- it: Changes C, Files F, Browser B, Terminal T, History H, Pull requests
-- P. A kind opens once: asking again brings its tab to the front.
--
-- Its toggle is the title bar's (the corner card at the window's right,
-- Ctrl+\); Ctrl+J, Ctrl+P, Ctrl+Shift+B and Ctrl+` open their tab, or
-- close the panel when it's already in front. Its edge drags its width.

local PanelHome = require("@ds/panel_home")
local IconButton = require("@ds/icon_button").IconButton
local Icon = require("@ds/icon")
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

local fixture = store.fixture ~= ""
local function kept<T>(key: string, default: T): State<T>
	return if fixture then ui.state(key, default) else ui.persist(key, default)
end

-- Open or not lives with Changes (its refresh listens).
M.open = changes.open
-- The tabs open, by kind, in their order; the one in front ("" the home).
M.tabs = kept("panel.tabs", {} :: { string })
M.tab = kept("panel.front", "" :: string)
M.width = kept("panel.width", 460)

type Kind = { id: string, icon: string, label: string, key: string, build: () -> Node }
local KINDS: { Kind } = {
	{ id = "changes", icon = "file-diff", label = "Changes", key = "C", build = changes.list },
	{ id = "files", icon = "folder-with-files", label = "Files", key = "F", build = files.view },
	{ id = "browser", icon = "global", label = "Browser", key = "B", build = browser.view },
	{ id = "terminal", icon = "terminal", label = "Terminal", key = "T", build = terminal.view },
	{ id = "history", icon = "git-branch", label = "History", key = "H", build = history.view },
	{ id = "pulls", icon = "pull-request", label = "Pull requests", key = "P", build = pulls.view },
}
local BY: { [string]: Kind } = {}
for _, k in KINDS do
	BY[k.id] = k
end

-- Its tab in front (opened if it isn't), and the panel open.
function M.show(kind: string)
	if not BY[kind] then
		return
	end
	local list = M.tabs:peek()
	if not table.find(list, kind) then
		local next = table.clone(list)
		table.insert(next, kind)
		M.tabs:set(next)
	end
	M.tab:set(kind)
	M.open:set(true)
end

-- A key's tab: shown, or (in front already) the panel closed.
function M.toggle(kind: string)
	if M.open:peek() and M.tab:peek() == kind then
		M.open:set(false)
	else
		M.show(kind)
	end
end

function M.toggle_panel()
	M.open:set(not M.open:peek())
end

local function close(kind: string)
	local list = M.tabs:peek()
	local i = table.find(list, kind)
	if not i then
		return
	end
	local next = table.clone(list)
	table.remove(next, i)
	M.tabs:set(next)
	if M.tab:peek() == kind then
		-- The tab beside it comes to the front; with none, the home.
		M.tab:set(next[math.min(i, #next)] or "")
	end
end

-- Development: a fixture may ask for a tab.
local fx = (store :: any).fixture_data
if fx and fx.panel_tab == "home" then
	M.tab:set("")
	M.open:set(true)
elseif fx and fx.panel_tab then
	M.show(fx.panel_tab)
end

-- ── The strip ──────────────────────────────────────────────────────────

local tab_button = ui.component(function(props: { kind: string }): Node
	local kind = props.kind
	local k = BY[kind]
	local over = ui.use_state(false)
	local function front(): boolean
		return M.tab:get() == kind
	end
	return ui.row {
		h = 28,
		pl = "$space.2",
		pr = "$space.1",
		gap = "$kx.space.tight",
		items = "center",
		shrink = 0,
		rounded = "$radius.lg",
		cursor = "pointer",
		label = k.label,
		bg = function()
			return if front() then "$zeron.selected.flat.bg" else "#00000000"
		end,
		hover = { bg = "$hover" },
		color = function()
			return if front() then "$text.primary" else "$text.secondary"
		end,
		on_hover = function(inside: boolean)
			over:set(inside)
		end,
		on_click = function()
			M.tab:set(kind)
		end,
		Icon { name = k.icon :: any, size = 14 },
		ui.text { k.label, literal = true, text_size = "$font.size.sm", weight = "$font.weight.medium" },
		-- × under the pointer (or in front); else room for it, still.
		ui.row {
			w = 18,
			h = 18,
			items = "center",
			justify = "center",
			rounded = "$radius.md",
			opacity = function(): number
				return if over:get() or front() then 1 else 0
			end,
			hover = { bg = "$hover" },
			color = "$text.tertiary",
			label = `Close {k.label}`,
			on_click = function()
				close(kind)
			end,
			Icon { name = "close", size = 12 },
		},
	}
end)

local tabs_view = ui.component(function(): Node
	return ui.row {
		gap = "$kx.space.hair",
		items = "center",
		min_w = 0,
		shrink = 1,
		overflow = "hidden",
		ui.each(M.tabs, function(kind: string): Node
			return tab_button { kind = kind }
		end, {
			key = function(kind: string)
				return kind
			end,
			direction = "row",
			gap = "$kx.space.hair",
			enter = { opacity = { 0, 1 }, duration = 140 },
		}),
		IconButton {
			label = "Open a tab",
			tooltip = "Open a tab",
			w = 28,
			h = 28,
			on_click = function()
				M.tab:set("")
			end,
			Icon { name = "plus", size = 14 },
		},
	}
end)

-- ── The home ───────────────────────────────────────────────────────────

-- The open thread's place: its project and branch.
-- The project's other threads (Zeron's lanes): the latest few, each a click
-- away.
local siblings = ui.computed(function(): { store.Thread }
	local t = store.thread:get()
	if not t then
		return {}
	end
	local project = nil
	for _, th in store.threads:get() do
		if th.id == t.id then
			project = th.project_id
		end
	end
	local out = {}
	for _, th in store.threads:get() do
		if th.project_id == project and th.id ~= t.id then
			table.insert(out, th)
			if #out >= 5 then
				break
			end
		end
	end
	return out
end)

local function sibling_row(th: store.Thread): Node
	return PanelHome.Row {
		icon = kx.AgentMark { agent = th.harness_id, size = 14 },
		label = th.title,
		tooltip = "Open this thread",
		on_click = function()
			require("@screens/nav").go("thread")
			store.open(th.id)
		end,
		trail = if th.status == "running"
			then kx.Spinner { size = 12 }
			elseif th.status == "needs_you" then ui.div { w = 6, h = 6, rounded = "$radius.full", bg = "$kx.attention" }
			else nil,
	}
end

local place = ui.computed(function(): { project: string, branch: string }?
	local t = store.thread:get()
	if not t then
		return nil
	end
	for _, th in store.threads:get() do
		if th.id == t.id then
			return { project = th.project, branch = th.branch or "" }
		end
	end
	return nil
end)

local function home(): Node
	local choices: { Node } = {}
	for _, k in KINDS do
		table.insert(choices, PanelHome.Choice {
			id = `panel-open-{k.id}`,
			icon = k.icon,
			label = k.label,
			key = k.key,
			on_click = function()
				M.show(k.id)
			end,
		})
	end
	return ui.scroll {
		grow = 1,
		min_h = 0,
		w_full = true,
		ui.column {
			w_full = true,
			px = "$space.2",
			pb = "$space.4",
			gap = "$kx.space.hair",
			ui.show(function()
				return place:get() ~= nil
			end, ui.column {
				w_full = true,
				gap = "$kx.space.hair",
				pb = "$space.3",
				PanelHome.Heading { label = "This thread" },
				PanelHome.Row {
					icon = "folder",
					label = function()
						local p = place:get()
						return if p then p.project else ""
					end,
				},
				PanelHome.Row {
					icon = "git-branch",
					label = function()
						local p = place:get()
						return if p and p.branch ~= "" then p.branch else "Working in the project's folder"
					end,
					tooltip = "Its changes",
					on_click = function()
						M.show("changes")
					end,
					trail = ui.row {
						gap = "$space.1",
						items = "center",
						ui.text {
							function()
								return `+{changes.totals:get().added}`
							end,
							literal = true,
							tabular = true,
							text_size = "$font.size.sm",
							color = "$kx.syntax.inserted",
						},
						ui.text {
							function()
								return `−{changes.totals:get().removed}`
							end,
							literal = true,
							tabular = true,
							text_size = "$font.size.sm",
							color = "$kx.syntax.deleted",
						},
					},
				},
			}),
			ui.show(function()
				return #siblings:get() > 0
			end, ui.column {
				w_full = true,
				gap = "$kx.space.hair",
				pb = "$space.3",
				PanelHome.Heading { label = "In this project" },
				ui.each(siblings, sibling_row, {
					key = function(th: store.Thread)
						return `{th.id}:{th.status}`
					end,
					gap = "$kx.space.hair",
				}),
			}),
			PanelHome.Heading { label = "Open here" },
			table.unpack(choices),
		},
	}
end

-- The home's letters, while it shows and no field has the keys.
local function on_key(e: any): boolean?
	if M.tab:peek() ~= "" or e.editing or e.ctrl or e.alt or e.meta or e["repeat"] then
		return nil
	end
	local key = if type(e.key) == "string" then string.upper(e.key) else ""
	for _, k in KINDS do
		if k.key == key then
			M.show(k.id)
			return true
		end
	end
	return nil
end

-- ── The panel ──────────────────────────────────────────────────────────

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
				M.width:set(math.clamp(from_w - (e.x - from_x), 320, 960))
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

-- The room the title bar keeps at the strip's right: the window's buttons
-- over the panel's top right.
local function strip_right(): any
	local ok, right = pcall(function(): number
		return require("@ds/title_bar").window_buttons_right()
	end)
	-- (And the panel's own toggle, which the bar stands beside them.)
	return ui.raw(if ok and type(right) == "number" then right + 44 else 48, "clears the window's own buttons and the panel's toggle")
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
		on_key = on_key,
		edge(),
		-- The strip, on the title bar's line.
		ui.row {
			h = 40,
			w_full = true,
			pl = "$space.2",
			pr = strip_right,
			gap = "$space.1",
			items = "center",
			window_drag = true,
			tabs_view {},
			ui.div { grow = 1 },
			ui.show(function()
				return M.tab:get() == "changes"
			end, changes.summary()),
		},
		-- Only the tab in front is built (and reads what it shows).
		ui.presence(M.tab, function(tab: string): Node
			local k = BY[tab]
			if not k then
				return home()
			end
			return ui.column { grow = 1, min_h = 0, w_full = true, k.build() }
		end, { grow = 1, min_h = 0, w_full = true }),
	}
end

return M
