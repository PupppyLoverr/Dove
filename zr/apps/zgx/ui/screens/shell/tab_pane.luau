-- A tab of the panel's standing in the page's split, a pane among the chats
-- (screens/shell/split's `tab:` panes; screens/panel/groups keeps which
-- tabs are out). A card as a chat's pane is: its head (the tab's name, its
-- ×; dragged by it, it moves among the panes, or back onto the panel), and
-- under that the tab's own view, built as the panel builds it. Its × sends it
-- back to the panel: the tab itself closes only from there.
--
-- `TabPane { id = leaf, box = fn, expanded = fn, shown = fn }`: `box` where it
-- stands (split.place's), `expanded` whether the panel is over the page,
-- `shown` whether the split shows.

local ds = require("@ds")
local Glass = require("@ds/glass")
local splits = require("@app/screens/shell/split")
local groups = require("@app/screens/panel/groups")
local panel_state = require("@app/screens/panel/state")
local boundary = require("@app/screens/shell/boundary")

-- Its head's height (a chat pane's, where it has one).
local HEAD = 32

export type Props = { id: string, box: () -> any, expanded: () -> boolean, shown: () -> boolean }

return function(props: Props): Node
	local id = props.id
	local key, tab = groups.of_leaf(id)
	local function panel(): any
		return boundary.load("@app/screens/panel")
	end
	local function title(): string
		local p = panel()
		return if p and p.tab_title and key and tab then p.tab_title(key, tab) else ""
	end
	local held = ui.use_state(false)
	local head = ui.row {
		id = `split-head-{id}`,
		class = "split-head",
		w_full = true,
		h = HEAD,
		shrink = 0,
		pl = "$space.3",
		pr = "$space.1",
		gap = "$zeron.space.tight",
		items = "center",
		cursor = function(): string
			return if held:get() then "grabbing" else "grab"
		end,
		on_drag = function(): any
			held:set(true)
			splits.dragging:set(true)
			groups.dragging:set(true)
			return { split_pane = id, panel_tab = tab, panel_key = key }
		end,
		drag_preview = function(): Node
			local card: { [any]: any } = {
				h = HEAD,
				px = "$space.3",
				items = "center",
				color = "$text.primary",
				ds.FadeText { title(), max_w = 220, text_size = "$font.size.sm", weight = "$font.weight.medium" },
			}
			Glass.style(card, "menu")
			card.rounded = "$radius.md"
			return ui.row(card)
		end,
		drag_preview_anchor = { 0.5, 0 },
		drag_preview_offset = { 0, 18 },
		on_drag_end = function()
			held:set(false)
			splits.dragging:set(false)
			groups.dragging:set(false)
		end,
		ds.FadeText {
			title,
			grow = 1,
			min_w = 0,
			text_size = "$font.size.sm",
			weight = "$font.weight.medium",
			color = "$text.secondary",
		},
		ds.IconButton {
			id = `split-close-{id}`,
			label = "Back to the panel",
			size = "sm",
			tooltip_side = "bottom",
			on_click = function()
				splits.discard(id)
				-- (Back in the panel, it's the tab showing there.)
				if key and tab then
					panel_state.activate(key, tab)
				end
			end,
			ds.Icon { name = "close-thin", size = 14 },
		},
	}
	local card = ui.column {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		overflow = "hidden",
		bg = "$zeron.inset.inactive",
		rounded = "$zeron.inset.radius",
		border = 1,
		border_color = "$zeron.inset.edge",
		head,
		-- (Built again with each panel made: the view is the panel showing's.)
		ui.presence(function(): string
			return tostring(groups.bodies:get())
		end, function(_made: string): Node
			local p = panel()
			local built = if p and p.tab_body and key and tab then p.tab_body(key, tab) else nil
			if built then
				return built
			end
			return ds.EmptyState { icon = "widget", size = "sm", fill = true, title = "This tab isn't open" }
		end, { w_full = true, grow = 1, min_h = 0 }),
	}
	local item: { [any]: any } = {
		id = `split-pane-{id}`,
		class = "split-pane tab-pane",
		pointer_events = "none",
		opacity = function(): number
			return if props.expanded() then 0 else 1
		end,
		ui.show(props.shown, card),
	}
	splits.place(item, props.box)
	return ui.column(item)
end
