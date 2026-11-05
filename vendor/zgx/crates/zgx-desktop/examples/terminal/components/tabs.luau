-- The titlebar: the tab strip (each tab an icon per pane and a name),
-- the slot a dragged pane would open in, and the tools.
local theme = require("../theme")
local icons = require("../icons")
local state = require("../state")
local sessions = require("../sessions")
local actions = require("../actions")
local sidebar = require("./sidebar")
local controls = require("./controls")
local menu = require("./menu")
local kit = require("./kit")
local layout = require("@lib/layout")
local frames = require("../frames")

local tabs, active, focus = state.tabs, state.active, state.focus
local dragging, tab_drop, tab_slot = state.dragging, state.tab_drop, state.tab_slot

-- A tab's shape, shared by the tab, the chip that follows the pointer while
-- one is dragged, and the ghost of the tab a dragged pane would open as.
local tab_style = ui.variants {
	base = {
		h = 28,
		max_w = 210,
		items = "center",
		pl = "$terminal.tab.inset",
		pr = "$terminal.tab.end",
		gap = "$terminal.tab.gap",
		rounded = "$terminal.tab.radius",
		shrink = 1,
		cursor = "default",
	},
	variants = {
		kind = {
			tab = {},
			-- A tab a dragged tab would drop on: it lights.
			target = { bg = "$terminal.drop" },
			-- Blurred behind, as the strip's glass is, so it reads over a terminal.
			chip = { font = theme.font, bg = theme.wash, backdrop_blur = 24, border = 1, border_color = theme.line },
			slot = {
				w = 132,
				shrink = 0,
				overflow = "hidden",
				bg = "$terminal.slot",
				border = 1,
				border_color = "$terminal.slot_line",
			},
		},
	},
	defaults = { kind = "tab" },
}

-- A pane's icon in a tab: on a little tile of its own, tinted for agents.
local function badge(pane: string, dim: () -> boolean): Node
	local desc = function()
		return sessions.describe(pane)
	end
	return ui.row {
		variants = kit.tile { dim = dim },
		w = 20,
		-- An agent's tile takes its own colour, from its session.
		bg = function()
			local tint = desc().tint
			if tint then
				return kit.tint(tint, if dim() then "1f" else "2e")
			end
			return if dim() then "$terminal.tile_dim" else "$terminal.tile_bg"
		end,
		border_color = function()
			local tint = desc().tint
			return if tint then kit.tint(tint, "33") else "$terminal.tile_line"
		end,
		controls.glyph(desc, 12, dim),
	}
end

-- More panes than fit: "+3" on a tile like the icons'.
local function more_badge(count: number, dim: () -> boolean): Node
	return ui.row {
		variants = kit.tile { dim = dim },
		px = "$terminal.tab.tile_pad",
		ui.text {
			`+{count}`,
			text_size = "$terminal.tab.tile_text",
			weight = "$font.weight.semibold",
			color = function()
				return if dim() then theme.tertiary else theme.secondary
			end,
		},
	}
end

-- A tab's icon: a pane's, or the count of those that do not fit.
type Badge = { key: string, pane: string?, more: number? }

-- A tab shows an icon per pane, three at most: past that, two and "+n".
local function tab_slots(id: string): State<{ Badge }>
	return ui.computed(function()
		local root: layout.Tree? = nil
		for _, tab in tabs:get() do
			if tab.id == id then
				root = tab.root
			end
		end
		local panes = if root then layout.leaves(root) else {}
		local slots: { Badge } = {}
		for i, pane in panes do
			if #panes > 3 and i == 3 then
				table.insert(slots, { key = "more", more = #panes - 2 })
				break
			end
			table.insert(slots, { key = pane, pane = pane })
		end
		return slots
	end)
end

-- A tab's icons, `dim` while it is not active.
local function badges(id: string, dim: () -> boolean): Node
	return ui.each(tab_slots(id), function(slot: Badge)
		if slot.more then
			return more_badge(slot.more, dim)
		end
		return badge(slot.pane or "", dim)
	end, {
		key = function(slot: Badge)
			return if slot.more then `more{slot.more}` else slot.key
		end,
	})
end

local function never(): boolean
	return false
end

-- A pane's name, "Terminal" until it has one.
local function name_of(pane: string?): string
	local name = sessions.describe(pane).name
	return if name ~= "" then name else "Terminal"
end

local M = {}

-- What follows the pointer while dragging a tab or a pane: the tab as it
-- shows active in the strip (a pane alone, its own icon).
function M.chip(pane: string?, tab: string?): Node
	local shown = if tab then badges(tab, never) else (if pane then badge(pane, never) else nil)
	return ui.row {
		variants = tab_style { kind = "chip" },
		ui.row { gap = "$terminal.tab.tile_gap", shrink = 0, items = "center", shown },
		ui.text {
			function()
				return name_of(pane)
			end,
			shrink = 1,
			min_w = 0,
			text_size = "$terminal.type.body",
			truncate = true,
			color = theme.text,
		},
	}
end

-- A dragged tab: a small window of what it holds, as a window's preview
-- shows it. Its title row is the tab's (its icons and name); under it, the
-- panes laid out as they are, in proportion, each a picture of what it shows
-- (taken as the drag starts), or its icon and name where there is none.
-- A dragged pane's picture: this much of its size, no narrower than PANE_MIN.
local PANE_SCALE, PANE_MIN = 0.22, 140
local MINI = {
	width = ui.theme.get("terminal.mini.width") :: number,
	max_h = ui.theme.get("terminal.mini.max_h") :: number,
	pad = ui.theme.get("terminal.mini.pad") :: number,
	gap = ui.theme.get("terminal.mini.pane_gap") :: number,
}
-- The box a `w` by `h` area shows in: as wide as the window allows, no
-- taller than it allows.
local function fit(w: number, h: number): (number, number)
	local inner = MINI.width - MINI.pad * 2
	if w <= 0 or h <= 0 then
		return inner, math.floor(inner * 0.6)
	end
	local scale = math.min(inner / w, MINI.max_h / h)
	return math.floor(w * scale), math.floor(h * scale)
end
-- A pane at `r` in the window: a picture of it, or its icon and name.
local function mini_pane(id: string, r: layout.Rect): Node
	-- At twice its size here, for a sharp picture on a Retina screen: a
	-- terminal's last frame, or a panel's card as the window last showed it
	-- (while its tab shows). A page is the platform's to draw, so it shows
	-- as its name.
	local wide = math.ceil(r.w * 2)
	local picture: string? = nil
	-- A panel's picture is its whole card, border and all.
	local carded = false
	local term = sessions.session(id)
	if term then
		picture = term:snapshot(wide)
	elseif not sessions.is_browser(id) then
		local card = frames.pane_element(id)
		picture = if card then card:snapshot(wide) else nil
		carded = picture ~= nil
	end
	return ui.row {
		absolute = true,
		left = r.x,
		top = r.y,
		w = r.w,
		h = r.h,
		items = "center",
		justify = "center",
		gap = "$terminal.tab.gap",
		overflow = "hidden",
		rounded = "$terminal.mini.pane_radius",
		bg = if carded then nil else theme.pane,
		border = if carded then 0 else 1,
		border_color = theme.line,
		if picture
			then ui.image {
				src = picture,
				absolute = true,
				left = 0,
				top = 0,
				w = r.w,
				h = r.h,
				fit = "cover",
			}
			else nil,
		if picture then nil else badge(id, never),
		if picture
			then nil
			else ui.text {
				name_of(id),
				shrink = 1,
				min_w = 0,
				truncate = true,
				text_size = "$terminal.type.body",
				color = theme.secondary,
			},
	}
end
-- The window: a title row of `icons` and `name`, then `panes` in a `w` by
-- `h` box.
local function mini_window(icons: Node, name: string, w: number, h: number, panes: { Node }): Node
	return ui.column {
		p = "$terminal.mini.pad",
		gap = "$terminal.mini.gap",
		rounded = "$terminal.mini.radius",
		font = theme.font,
		bg = "$terminal.mini.bg",
		-- Blurred behind (AppKit's material, in the preview's own window).
		backdrop_blur = 24,
		border = 1,
		border_color = theme.line,
		ui.row {
			w = w,
			h = 22,
			gap = "$terminal.tab.gap",
			items = "center",
			ui.row { gap = "$terminal.tab.tile_gap", shrink = 0, items = "center", icons },
			ui.text {
				name,
				shrink = 1,
				min_w = 0,
				truncate = true,
				text_size = "$terminal.type.body",
				color = theme.text,
			},
		},
		ui.div { relative = true, w = w, h = h, table.unpack(panes) },
	}
end
function M.mini(tab: string): Node
	local area = frames.area()
	local w, h = fit(area.w, area.h)
	local panes: { Node } = {}
	local root = actions.root_of(tab)
	if root then
		local placed = layout.frames(root, state.ratios:peek(), MINI.gap).panes
		for _, id in layout.leaves(root) do
			table.insert(panes, mini_pane(id, layout.resolve(placed[id], w, h)))
		end
	end
	return mini_window(badges(tab, never), name_of((focus:peek())[tab]), w, h, panes)
end
-- A pane dragged by its grip: a picture of it alone, about a fifth of its
-- size (as Ghostty's split drag shows one), on the window's glass.
function M.mini_pane(pane: string): Node
	local shown = frames.shown(frames.pane_element(pane))
	local w = math.clamp(math.floor(shown.w * PANE_SCALE), PANE_MIN, MINI.width)
	local h = if shown.w > 0 then math.floor(w * shown.h / shown.w) else math.floor(w * 0.6)
	if h > MINI.max_h then
		w, h = math.floor(w * MINI.max_h / h), MINI.max_h
	end
	return ui.div {
		relative = true,
		w = w,
		h = h,
		rounded = "$terminal.mini.pane_radius",
		font = theme.font,
		bg = "$terminal.mini.bg",
		backdrop_blur = 24,
		mini_pane(pane, { x = 0, y = 0, w = w, h = h }),
	}
end

-- A tab's padding, for the animations that fold it away and back.
local INSET, END = ui.theme.get("terminal.tab.inset"), ui.theme.get("terminal.tab.end")

local function tab_button(id: string): Node
	local function pane(): string?
		return (focus:get())[id]
	end
	local function is_active()
		return active:get() == id
	end
	local hover_layer, hover = controls.hover_glass("$terminal.tab.radius")
	-- Lifted out of the strip, it folds away (still mounted: unmounting the
	-- dragged tab would end the drag), and unfolds where it lands.
	local element: Element? = nil
	local width = 0
	local SPACE = { "width", "minWidth", "paddingLeft", "paddingRight", "marginRight", "opacity" }
	ui.effect(function()
		local out = state.lifted:get() == id
		local el = element
		if not el then
			return
		end
		if out then
			width = el.width or 0
			anime.animate(el, {
				minWidth = 0,
				width = 0,
				paddingLeft = 0,
				paddingRight = 0,
				-- Its gap in the strip closes too.
				marginRight = -3,
				opacity = 0,
				duration = 220,
				ease = "outQuint",
			})
		elseif width > 0 then
			local d = dragging:peek()
			anime.animate(el, {
				width = width,
				paddingLeft = INSET,
				paddingRight = END,
				marginRight = 0,
				opacity = if d.kind == "tab" and d.id == id then 0.35 else 1,
				duration = 240,
				ease = "outQuint",
				onComplete = function()
					anime.utils.cleanInlineStyles(el, SPACE)
				end,
			})
			width = 0
		end
	end)
	return ui.row {
		variants = tab_style {
			kind = function()
				return if tab_drop:get() == id then "target" else "tab"
			end,
		},
		relative = true,
		on_mount = function(el: Element)
			element = el
		end,
		-- Where it was while dragged: a dim slot.
		opacity = function()
			local d = dragging:get()
			return if d.kind == "tab" and d.id == id then 0.35 else 1
		end,
		on_click = function()
			actions.activate(id)
		end,
		context_menu_style = menu.STYLE,
		context_menu_below = true,
		context_menu = function()
			local others = #tabs:peek() > 1
			local items = {
				menu.entry("New Tab", function()
					actions.new_tab(sessions.cwd_of((focus:peek())[id]))
				end),
				menu.separator(),
				menu.entry("Close Tab", function()
					actions.close_tab(id)
				end),
			}
			if others then
				table.insert(
					items,
					menu.entry("Close Other Tabs", function()
						for _, other in tabs:peek() do
							if other.id ~= id then
								actions.close_tab(other.id)
							end
						end
					end)
				)
			end
			return items
		end,
		on_hover = hover,
		on_drag = function(): state.Payload
			dragging:set { kind = "tab", id = id }
			return { kind = "tab", id = id }
		end,
		drag_preview = function()
			return M.mini(id)
		end,
		-- Below the pointer, so the strip and the panes it is over show.
		drag_preview_anchor = { 0.5, 0 },
		drag_preview_offset = { 0, 18 },
		on_drag_end = function()
			dragging:set {}
			state.drop:set {}
			tab_drop:set("")
			-- Let go away from the strip and from another tab's panes: it is
			-- back where it was.
			actions.land(id)
		end,
		drop_when = function(payload: state.Payload)
			return payload.kind == "pane" or (payload.kind == "tab" and payload.id ~= id)
		end,
		-- A tab or a pane over its middle marks it: dropping moves the pane,
		-- or the tab's panes, in, and resting there shows it (spring-loaded,
		-- below). Near an edge, a tab takes this one's place as it passes (a
		-- lifted one lands), and a pane opens a new tab beside it.
		on_drag_over = function(payload: state.Payload, e)
			if payload.kind == "tab" then
				actions.land(payload.id)
				-- Its outer quarters reorder, as a link's drop does in Chrome's
				-- strip; its middle half is the tab.
				local quarter = e.w / 4
				if e.local_x < quarter or e.local_x > e.w - quarter then
					if tab_drop:peek() == id then
						tab_drop:set("")
					end
					actions.reorder(payload.id, id, if e.local_x < quarter then "before" else "after")
				elseif tab_drop:peek() ~= id then
					tab_drop:set(id)
				end
				return
			end
			-- Its edges (thin, as the gaps between tabs are) open a new tab
			-- beside it; the rest is the tab, which springs open.
			local edge = math.min(10, e.w * 0.15)
			if e.local_x < edge then
				actions.show_slot(payload.id, id)
			elseif e.local_x > e.w - edge then
				local list = tabs:peek()
				local after = ""
				for i, other in list do
					if other.id == id and list[i + 1] then
						after = list[i + 1].id
					end
				end
				actions.show_slot(payload.id, after)
			else
				actions.hide_slot()
				if tab_drop:peek() ~= id then
					tab_drop:set(id)
				end
			end
		end,
		on_drag_leave = function()
			if tab_drop:peek() == id then
				tab_drop:set("")
			end
		end,
		-- Spring-loaded: a pane or a tab resting on another tab's middle (the
		-- system's delay, or Space) shows that tab, so what is dragged can go
		-- among its panes. As Finder's folders do, it flashes twice first.
		on_drag_spring = function(payload: state.Payload)
			local el = element
			if tab_drop:peek() ~= id or active:peek() == id or not el then
				return false
			end
			anime.animate(el, {
				opacity = { 1, 0.35 },
				duration = 70,
				alternate = true,
				loop = 3,
				onComplete = function()
					anime.utils.cleanInlineStyles(el, { "opacity" })
					actions.activate(id)
				end,
			})
			return true
		end,
		on_drop = function(payload: state.Payload)
			local marked = tab_drop:peek() == id
			tab_drop:set("")
			if payload.kind == "pane" and not actions.commit_slot() then
				actions.move_pane_to_tab(payload.id, id)
			elseif payload.kind == "tab" and marked then
				actions.join_tab(payload.id, id)
			end
		end,
		-- The active tab's frosted glass (one piece, sliding between tabs)
		-- over a fainter glass while the pointer is on the tab.
		hover_layer,
		controls.glass("tab-active", is_active, "$terminal.tab.radius"),
		ui.row {
			gap = "$terminal.tab.tile_gap",
			shrink = 0,
			items = "center",
			badges(id, function()
				return not is_active()
			end),
		},
		ui.text {
			function()
				return name_of(pane())
			end,
			shrink = 1,
			min_w = 0,
			text_size = "$terminal.type.body",
			truncate = true,
			color = function()
				return if is_active() then theme.text else theme.secondary
			end,
		},
		-- The bell rang in a pane of a background tab.
		ui.div {
			hidden = function()
				local p = pane()
				return is_active() or p == nil or not sessions.is_terminal(p) or not ui.term(p).bell
			end,
			w = 6,
			h = 6,
			rounded = "$radius.full",
			bg = theme.bell,
			shrink = 0,
		},
	}
end

-- The tab strip's items: the tabs, and the slot a dragged pane would open
-- in (`slot` is that pane).
type Item = { id: string, slot: string? }
local strip = ui.computed(function(): { Item }
	local list = tabs:get()
	local s = tab_slot:get()
	local out: { Item } = {}
	for _, tab in list do
		if s.pane and tab.id == s.before then
			table.insert(out, { id = "slot", slot = s.pane })
		end
		table.insert(out, { id = tab.id })
	end
	if s.pane and (s.before == "" or #out == #list) then
		table.insert(out, { id = "slot", slot = s.pane })
	end
	return out
end)

-- Where a dragged pane would open as a tab: a ghost of that tab, opening
-- up so the tabs after it slide over.
local function slot_tab(pane: string): Node
	return ui.row {
		variants = tab_style { kind = "slot" },
		on_mount = function(el)
			anime.animate(el, { width = { 0, 132 }, opacity = { 0, 1 }, duration = 200, ease = "outQuint" })
		end,
		drop_when = function(payload: state.Payload)
			return payload.kind == "pane"
		end,
		on_drop = function()
			actions.commit_slot()
		end,
		badge(pane, never),
		ui.text {
			function()
				return name_of(pane)
			end,
			shrink = 1,
			min_w = 0,
			text_size = "$terminal.type.body",
			truncate = true,
			color = theme.secondary,
		},
	}
end

function M.Titlebar(): Node
	return ui.row {
		w_full = true,
		h = theme.titlebar,
		items = "center",
		-- A pane anywhere else on it (the new tab button, the tools) opens as
		-- a tab where the slot shows, at the end if none does yet. A lifted
		-- tab back on it lands.
		drop_when = function(payload: state.Payload)
			return payload.kind == "pane" or payload.kind == "tab"
		end,
		on_drag_over = function(payload: state.Payload)
			if payload.kind == "tab" then
				actions.land(payload.id)
			elseif not (tab_slot:peek()).pane then
				actions.show_slot(payload.id, "")
			end
		end,
		on_drop = function(payload: state.Payload)
			if payload.kind == "pane" then
				actions.commit_slot()
			end
		end,
		gap = "$terminal.tab.strip_gap",
		pr = "$terminal.inset",
		-- Room for the traffic lights and the sidebar toggle (drawn over this,
		-- app.luau) while the sidebar is closed; a little air after an open
		-- one. It eases as the sidebar does, so the tabs slide clear of the
		-- toggle rather than under it.
		controls.drag_space {
			w = function()
				return if state.sidebar:get() then 6 else sidebar.toggle_left() + sidebar.TOGGLE_SIZE
			end,
			layout = "width",
			layout_ease = sidebar.EASE,
			layout_duration = sidebar.DURATION,
			shrink = 0,
		},
		ui.row {
			gap = "$terminal.tab.strip_gap",
			items = "center",
			shrink = 1,
			min_w = 0,
			ui.each(strip, function(item: Item)
				if item.slot then
					return slot_tab(item.slot)
				end
				return tab_button(item.id)
			end, {
				key = function(item: Item)
					return item.id
				end,
				-- A reorder slides the tabs from where they were.
				animate = { duration = 240, ease = "outQuint" },
			}),
		},
		controls.tool(icons.plus, function()
			actions.new_tab(sessions.cwd_of(actions.current()))
		end),
		-- Dropping a pane here gives it a tab of its own.
		controls.drag_space {
			grow = 1,
			min_w = 16,
			drop_when = function(payload: state.Payload)
				return payload.kind == "pane"
			end,
			on_drag_over = function(payload: state.Payload)
				actions.show_slot(payload.id, "")
			end,
			on_drop = function()
				actions.commit_slot()
			end,
		},
		controls.tool(icons.split_right, function()
			actions.split("row")
		end),
		controls.tool(icons.split_down, function()
			actions.split("column")
		end),
	}
end

return M
