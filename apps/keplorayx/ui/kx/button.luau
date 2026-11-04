--!strict
-- Buttons, on Zeron's recipe (ds/button, MIT; see THIRD_PARTY_NOTICES.md):
-- 8-point corners (6 small) unless `shape = "pill"`. `Button { "Continue",
-- on_click = … }` is the way on, one per screen, the lightest thing on the
-- glass; `tone` picks its weight. Every tone says its own hover and press,
-- and focus is a ring, never a shadow.
--
--   Button { tone = "secondary", "Not now" }
--   Button { tone = "ghost", size = "sm", Icon { name = "plus" }, "New" }
--   Button { icon = true, label = "Close", Icon { name = "close" } }
--   Button { busy = saving, "Save" }      the ring in the label's place

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
			primary = {
				bg = "$kx.primary.bg",
				color = "$kx.primary.text",
				hover = { bg = "$kx.primary.hover" },
				active = { bg = "$kx.primary.bg", opacity = 0.88 },
			},
			secondary = {
				bg = "$kx.secondary.bg",
				color = "$text.primary",
				hover = { bg = "$kx.secondary.hover" },
				active = { bg = "$pressed" },
			},
			ghost = {
				bg = "#00000000",
				color = "$text.secondary",
				hover = { bg = "$kx.secondary.bg", color = "$text.primary" },
				active = { bg = "$kx.secondary.hover", color = "$text.primary" },
			},
			-- Keplorayx's own light (or your star's), for a chosen thing.
			accent = {
				bg = "$accent",
				color = "$accent.text",
				hover = { bg = "$accent.hover" },
				active = { bg = "$accent", opacity = 0.88 },
			},
			-- What needs you: the warm colour, spent only here.
			attention = {
				bg = "$kx.attention",
				color = "$kx.primary.text",
				hover = { opacity = 0.92 },
				active = { opacity = 0.84 },
			},
			danger = {
				bg = "$kx.danger.soft",
				color = "$danger",
				hover = { bg = "$kx.danger.hover" },
				active = { bg = "$kx.danger.active" },
			},
		},
		size = {
			xs = { h = 24, px = "$kx.space.tight", rounded = "$radius.md", text_size = "$font.size.sm" },
			sm = { h = 28, px = "$kx.space.snug", rounded = "$radius.md", text_size = "$font.size.sm" },
			md = { h = "$control.height", px = "$space.3", rounded = "$radius.lg", text_size = "$font.size.md" },
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

export type Tone = "primary" | "secondary" | "ghost" | "accent" | "attention" | "danger"
export type Size = "xs" | "sm" | "md" | "lg"
export type Props = Merge<ContainerProps, {
	tone: (Tone | () -> Tone)?,
	size: Size?,
	shape: ("rounded" | "pill")?,
	icon: boolean?,
	-- Working: the ring in the label's place, the label kept unseen so the
	-- width holds, and clicks ignored. Never "Saving…".
	busy: (State<boolean> | () -> boolean)?,
	[number]: Child,
}>

local INK: { [string]: string } = {
	primary = "$kx.primary.text",
	secondary = "$text.primary",
	ghost = "$text.secondary",
	accent = "$accent.text",
	attention = "$kx.primary.text",
	danger = "$danger",
}
local SPIN: { [string]: number } = { xs = 12, sm = 12, md = 14, lg = 16 }

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
	local status = require("@kx/status")
	local function busy(): boolean
		return if type(given) == "function" then given() == true else (given :: any):get() == true
	end
	local label: { [any]: any } = {
		items = "center",
		gap = "$space.2",
		opacity = function(): number
			return if busy() then 0 else 1
		end,
	}
	for i, child in ipairs(props :: any) do
		label[i] = child
		p[i] = nil
	end
	local tone = if type(props.tone) == "string" then props.tone else "primary"
	p.relative = true
	p[1] = ui.row(label)
	p[2] = ui.row {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		items = "center",
		justify = "center",
		pointer_events = "none",
		ui.show(busy, status.Spinner { size = SPIN[props.size or "md"], ink = INK[tone :: string] }),
	}
	local click = props.on_click
	if click then
		p.on_click = function(...)
			if not busy() then
				click(...)
			end
		end
	end
	return ui.button(p)
end

return M
