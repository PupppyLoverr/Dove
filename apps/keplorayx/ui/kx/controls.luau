--!strict
-- Controls of Orbit's own.
--
--   Field { value = state, placeholder = "Name", label = "Your name" }
--   Switch { value = state, label = "Reduce motion" }
--   Segmented { value = state, options = { { label, value } }, width = 88 }
--       a recessed track and a raised puck that glides to the choice
--   Pill { … }   the Ray bar's and the composer's capsule: an even inset,
--                and everything inside it concentric with its corner

local motion = require("@kx/motion")
local shape = require("@kx/shape")
local surface = require("@kx/surface")

local M = {}

type P = { [any]: any }

function M.Field(props: P): Node
	local p = table.clone(props)
	p.h = p.h or "$control.height"
	p.px = p.px or "$space.3"
	p.rounded = p.rounded or "$radius.md"
	p.bg = p.bg or "$surface.base"
	p.border = 1
	p.border_color = "$border.subtle"
	p.placeholder_color = "$text.tertiary"
	p.caret_color = "$accent"
	p.text_size = p.text_size or "$font.size.md"
	p.focus = { border_color = "$focus.ring" }
	p.hover = { border_color = "$border" }
	return ui.input(p)
end

local TRACK_W, TRACK_H, KNOB, INSET = 34, 20, 16, 2

function M.Switch(props: P): Node
	local value: State<boolean> = props.value
	local travel = TRACK_W - KNOB - INSET * 2
	local x = ui.computed(function()
		return if value:get() then travel else 0
	end)
	local function flip()
		value:set(not value:peek())
		if props.on_change then
			props.on_change(value:peek())
		end
	end
	local track = ui.div {
		w = TRACK_W,
		h = TRACK_H,
		shrink = 0,
		relative = true,
		rounded = "$radius.full",
		role = "switch",
		focusable = true,
		checked = function()
			return value:get()
		end,
		label = props.label,
		cursor = "pointer",
		bg = function()
			return if value:get() then "$accent" else "$surface.raised"
		end,
		border = 1,
		border_color = "$border.subtle",
		hover = { border_color = "$border" },
		focus = { outline = 2, outline_color = "$focus.ring", outline_offset = 2 },
		on_click = flip,
		on_key = function(e: any)
			if e.key == " " or e.key == "Space" or e.key == "Enter" then
				flip()
				return true
			end
			return nil
		end,
		ui.div {
			absolute = true,
			top = INSET - 1,
			left = INSET - 1,
			w = KNOB,
			h = KNOB,
			rounded = "$radius.full",
			bg = "$kx.knob",
			shadow = "$shadow.sm",
			x = motion.tween(x, motion.move(travel)),
		},
	}
	if props.text == nil then
		return track
	end
	return ui.row {
		gap = "$space.3",
		items = "center",
		ui.column { grow = 1, gap = "$kx.space.hair", props.text, props.hint },
		track,
	}
end

function M.Segmented(props: P): Node
	local value: State<any> = props.value
	local options: { { label: string, value: any } } = props.options
	local width: number = props.width or 88
	local index = ui.computed(function()
		local v = value:get()
		for i, o in options do
			if o.value == v then
				return i - 1
			end
		end
		return 0
	end)
	local left = ui.computed(function()
		return index:get() * width
	end)
	local segments: { Node } = {}
	for _, o in options do
		table.insert(
			segments,
			ui.button {
				w = width,
				h = "100%",
				p = 0,
				justify = "center",
				bg = "#00000000",
				hover = { bg = "#00000000" },
				role = "radio",
				checked = function()
					return value:get() == o.value
				end,
				cursor = "pointer",
				on_click = function()
					value:set(o.value)
				end,
				ui.text {
					o.label,
					literal = true,
					text_size = "$font.size.sm",
					weight = "$font.weight.medium",
					color = function()
						return if value:get() == o.value then "$text.primary" else "$text.secondary"
					end,
				},
			}
		)
	end
	return ui.row {
		h = 28,
		w = width * #options + 4,
		self = "start",
		shrink = 0,
		p = "$kx.space.hair",
		rounded = "$radius.md",
		bg = "$surface.sunken",
		role = "radiogroup",
		label = props.label,
		focus_group = "horizontal",
		relative = true,
		-- The puck: raised, lit along its top, gliding under the choice.
		ui.div {
			absolute = true,
			top = 2,
			left = 2,
			w = width,
			h = 24,
			rounded = "$kx.segmented.puck",
			bg = "$surface.raised",
			inset_shadow = "$kx.lip.inset_strong",
			shadow = "$shadow.sm",
			x = motion.tween(left, motion.move(width)),
			aria_hidden = true,
		},
		table.unpack(segments),
	}
end

function M.Pill(props: P): Node
	local p = table.clone(props)
	shape.contain(p, { radius = "$kx.pill.radius", inset = "$kx.pill.inset", border = 1 })
	p.items = p.items or "end"
	p.gap = p.gap or "$space.1"
	p.bg = p.bg or "$surface.raised"
	surface.lip(p, true)
	p.shadow = p.shadow or "$shadow.md"
	return ui.row(p)
end

return M
