--!strict
-- The desktop's widgets, all on one surface (src/widgets.rs): a transparent
-- window over the desktop, under every app's window, never taking the keys.
-- Each widget sits where it was put (`x`, `y`: points from the desktop's top
-- left), at its variant's size, a card of glass (screens/widget_card).

ui.theme.strict("warn")
local kx = require("@kx")
kx.setup()

local Card = require("@screens/widget_card")

type Widget = { id: string, kind: string, variant: string, x: number?, y: number? }

local widgets: State<{ Widget }> = ui.shared("kx.widgets", {} :: { Widget })

-- Each variant's size, in points (src/widgets.rs `size`, the window region's).
local function size(kind: string, variant: string): (number, number)
	if variant == "small" then
		return 164, 164
	elseif variant == "large" then
		return 344, 344
	elseif kind == "calendar" and variant == "month" then
		return 344, 300
	elseif kind == "ask" then
		return 344, 120
	end
	return 344, 164
end

return ui.div {
	w_full = true,
	h_full = true,
	relative = true,
	ui.each(widgets, function(w: Widget): Node
		local width, height = size(w.kind, w.variant)
		return ui.div {
			absolute = true,
			left = ui.raw(w.x or 0, "where it was put"),
			top = ui.raw(w.y or 0, "where it was put"),
			w = width,
			h = height,
			p = 4,
			Card(w.id),
		}
	end, {
		key = function(w: Widget): string
			return `{w.id}:{w.kind}:{w.variant}:{w.x or 0}:{w.y or 0}`
		end,
	}),
}
