--!strict
-- Buttons. `Button { "Continue", on_click = … }` is the way on, one per
-- screen; `tone` picks its weight. Every tone says its own hover and press
-- (one wash, light over light), and focus is a ring, never a shadow.
--
--   Button { tone = "secondary", "Not now" }
--   Button { tone = "ghost", size = "sm", Icon { name = "plus" }, "New" }
--   Button { icon = true, label = "Close", Icon { name = "close" } }
--   Button { shape = "pill", … }        the Ray bar's and the composer's

local recipe = ui.variants {
	base = {
		items = "center",
		justify = "center",
		gap = "$space.2",
		font = "$font.sans",
		weight = "$font.weight.medium",
		cursor = "pointer",
		shrink = 0,
		disabled = { opacity = 0.4 },
		focus = { outline = 2, outline_color = "$focus.ring", outline_offset = 2 },
	},
	variants = {
		tone = {
			-- The way on: the app's own light.
			primary = {
				bg = "$accent",
				color = "$accent.text",
				hover = { bg = "$accent.hover" },
				active = { bg = "$accent", opacity = 0.88 },
			},
			-- Beside it: a raised fill that firms as it's touched.
			secondary = {
				bg = "$surface.raised",
				color = "$text.primary",
				border = 1,
				border_color = "$border.subtle",
				hover = { bg = "$surface.raised", border_color = "$border" },
				active = { bg = "$pressed" },
			},
			-- Back, Skip, a toolbar's: text until touched.
			ghost = {
				bg = "#00000000",
				color = "$text.secondary",
				hover = { bg = "$hover", color = "$text.primary" },
				active = { bg = "$pressed", color = "$text.primary" },
			},
			-- What needs you: the warm colour, spent only here.
			attention = {
				bg = "$kx.attention",
				color = "$accent.text",
				hover = { opacity = 0.92 },
				active = { opacity = 0.84 },
			},
			-- Delete, Discard: what can't be taken back.
			danger = {
				bg = "$kx.danger.soft",
				color = "$danger",
				hover = { bg = "$kx.danger.hover" },
				active = { bg = "$kx.danger.active" },
			},
		},
		size = {
			xs = { h = 24, px = "$kx.space.tight", rounded = "$radius.sm", text_size = "$font.size.sm" },
			sm = { h = 28, px = "$kx.space.snug", rounded = "$radius.md", text_size = "$font.size.sm" },
			md = { h = "$control.height", px = "$space.3", rounded = "$radius.md", text_size = "$font.size.md" },
			lg = { h = 40, px = "$space.5", rounded = "$radius.lg", text_size = "$font.size.lg" },
		},
		shape = { rounded = {}, pill = {} },
		icon = { [true] = {}, [false] = {} },
	},
	compound = {
		{ shape = "pill", rounded = "$radius.full" },
		{ icon = true, size = "xs", w = 24, px = 0 },
		{ icon = true, size = "sm", w = 28, px = 0 },
		{ icon = true, size = "md", w = "$control.height", px = 0 },
		{ icon = true, size = "lg", w = 40, px = 0 },
	},
	defaults = { tone = "primary", size = "md", shape = "rounded", icon = false },
}

export type Tone = "primary" | "secondary" | "ghost" | "attention" | "danger"
export type Size = "xs" | "sm" | "md" | "lg"
export type Props = Merge<ContainerProps, {
	tone: (Tone | () -> Tone)?,
	size: Size?,
	shape: ("rounded" | "pill")?,
	icon: boolean?,
	-- Working: the label kept (so the width holds) under a quiet pulse, and
	-- clicks ignored. Never "Saving…".
	busy: (State<boolean> | () -> boolean)?,
	[number]: Child,
}>

local motion = require("@kx/motion")

local M = {}

M.recipe = recipe

function M.Button(props: Props): Node
	local p = table.clone(props) :: any
	p.variants = recipe { tone = props.tone, size = props.size, shape = props.shape, icon = props.icon }
	p.tone, p.size, p.shape, p.icon, p.busy = nil, nil, nil, nil, nil
	local given = props.busy
	if given == nil then
		return ui.button(p)
	end
	local function busy(): boolean
		return if type(given) == "function" then given() == true else (given :: any):get() == true
	end
	local click = p.on_click
	p.on_click = function(...)
		if not busy() and click then
			click(...)
		end
	end
	local glow = motion.breath(0.55, 1)
	p.opacity = function(): number
		return if busy() then glow() else 1
	end
	return ui.button(p)
end

return M
