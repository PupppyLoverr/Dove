--!strict
-- What floats: menus, popovers, dialogs, tooltips and toasts, all one
-- glass (kx/surface `float_style`), all settling in on Orbit's curve.
--
--   Menu { open = state, anchor = Button { … }, MenuItem { "Rename", on_click = … } }
--   Popover { open = state, anchor = …, … }
--   Dialog { open = state, title = "Archive thread?", body = "…", actions = { … } }
--   Tooltip(node, "Search  ⌘K")         a node with a quiet hint
--   kx.toast { title = "Copied" }        one line, low and centred
--   Toasts {}                            once per window, last in its tree

local surface = require("@kx/surface")
local motion = require("@kx/motion")
local shape = require("@kx/shape")
local icon = require("@kx/icon")

local M = {}

type P = { [any]: any }

function M.Menu(props: P): Node
	local p = surface.float_style(table.clone(props))
	p.p = p.p or "$space.1"
	p.w = p.w or 232
	return ui.menu(p)
end

function M.MenuItem(props: P): Node
	local p = table.clone(props)
	p.rounded = "$kx.menu.row"
	p.h = p.h or 30
	p.px = p.px or "$space.2"
	p.gap = "$space.2"
	if p.icon then
		local name = p.icon
		p.icon = nil
		local label = p[1]
		-- An agent's or a brand's mark is drawn as itself.
		local mark = type(name) == "string" and (string.match(name, "%-mark$") ~= nil or string.sub(name, 1, 6) == "brand-")
		p[1] = if mark
			then icon.Icon { mark = name, size = 16 }
			else icon.Icon { name = name, size = 16, color = "$text.secondary" }
		p[2] = ui.text { label, literal = true, grow = 1 }
		p.label = p.label or label
	end
	return ui.menu_item(p)
end

function M.Popover(props: P): Node
	local p = surface.float_style(table.clone(props))
	p.p = p.p or "$space.3"
	return ui.popover(p)
end

function M.Dialog(props: P): Node
	local p = surface.float_style(table.clone(props))
	p.rounded = "$radius.xl"
	p.w = p.w or 400
	p.p = "$space.5"
	p.gap = "$space.3"
	local title, body, actions = p.title, p.body, p.actions
	p.title, p.body, p.actions = nil, nil, nil
	p.label = p.label or title
	local children: { any } = {}
	if title then
		table.insert(children, ui.text {
			title,
			literal = true,
			text_size = "$font.size.lg",
			line_height = "$font.line.lg",
			weight = "$font.weight.medium",
			heading = 2,
		})
	end
	if body then
		table.insert(children, ui.text {
			body,
			literal = true,
			wrap = true,
			color = "$text.secondary",
			line_height = "$font.line.md",
		})
	end
	for _, c in ipairs(p) do
		table.insert(children, c)
	end
	if actions then
		table.insert(children, ui.row { justify = "end", gap = "$space.2", mt = "$space.2", table.unpack(actions) })
	end
	for i = #p, 1, -1 do
		p[i] = nil
	end
	for i, c in children do
		p[i] = c
	end
	return ui.modal(p)
end

function M.Tooltip(node: Node, hint: string): Node
	return ui.div { tooltip = hint, tooltip_delay = 450, node }
end

-- ── Toasts ─────────────────────────────────────────────────────────────

export type Toast = { id: number, title: string, body: string?, kind: string? }

local toasts = ui.state("kx.toasts", {} :: { Toast })
local next_id = 0

function M.toast(t: { title: string, body: string?, kind: string?, duration: number? })
	-- (Past every id showing: each script that requires this module counts
	-- on its own, and two toasts with one id would be one row.)
	for _, x in toasts:peek() do
		next_id = math.max(next_id, x.id)
	end
	next_id += 1
	local id = next_id
	-- The same toast again replaces the one showing, not stacks on it.
	local list = {}
	for _, x in toasts:peek() do
		if x.title ~= t.title or x.body ~= t.body then
			table.insert(list, x)
		end
	end
	table.insert(list, { id = id, title = t.title, body = t.body, kind = t.kind })
	while #list > 3 do
		table.remove(list, 1)
	end
	toasts:set(list)
	-- An error stays longer; everything goes on its own.
	do
		ui.after(t.duration or if t.kind == "error" then 7000 else 3200, function()
			local now = {}
			for _, x in toasts:peek() do
				if x.id ~= id then
					table.insert(now, x)
				end
			end
			toasts:set(now)
		end)
	end
end

local function toast_row(t: Toast)
	local warm = t.kind == "attention" or t.kind == "error"
	return ui.row(surface.float_style {
		rounded = "$radius.full",
		px = "$space.4",
		h = 36,
		gap = "$space.2",
		items = "center",
		live = "polite",
		if warm then ui.div { w = 6, h = 6, rounded = "$radius.full", bg = if t.kind == "error" then "$danger" else "$kx.attention" } else nil,
		ui.text { t.title, literal = true, weight = "$font.weight.medium", text_size = "$font.size.sm", truncate = true },
		if t.body then ui.text { t.body, literal = true, color = "$text.secondary", text_size = "$font.size.sm", truncate = true } else nil,
	})
end

function M.Toasts(_props: P?): Node
	-- The whole window, its toasts at the foot (a column given only its
	-- foot laid them out at the top, under the notch).
	return ui.column {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		justify = "end",
		pb = "$space.8",
		items = "center",
		gap = "$space.2",
		pointer_events = false,
		ui.each(toasts, toast_row, {
			key = function(t: Toast)
				return t.id
			end,
			items = "center",
			gap = "$space.2",
			animate = { duration = motion.enter().duration, ease = motion.ENTER },
			enter = { opacity = { 0, 1 }, y = { 8, 0 }, duration = motion.enter().duration, ease = motion.ENTER },
			exit = { opacity = { 1, 0 }, duration = motion.exit().duration, ease = motion.EXIT },
		}),
	}
end

-- (The shape module, for screens that build their own floats.)
M.shape = shape

return M
