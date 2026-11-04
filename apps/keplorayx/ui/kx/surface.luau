--!strict
-- Surfaces: depth from light, not lines. A lifted surface is a little
-- brighter than what it sits on, and its top edge catches light (the lip,
-- a hairline brighter than the rest of its edge), as the notch's does.
--
--   Card { … }                     raised on the window, the lip on top
--   Card { level = "sunken", … }   set into it
--   Float { … }                    glass that floats: menus, popovers,
--                                  the Ray bar's results
--   float_style(props)             the same for zgx's own panels
--                                  (ui.popover, ui.menu, ui.modal)

local material = require("@kx/material")

local M = {}

type P = { [any]: any }

export type Level = "raised" | "base" | "sunken"

local BG: { [Level]: string } = {
	raised = "$surface.raised",
	base = "$surface.base",
	sunken = "$surface.sunken",
}

-- The lip: the top edge's own light (an inset highlight a point deep),
-- over a faint hairline all round.
function M.lip(props: P, strong: boolean?): P
	props.border = 1
	props.border_color = "$border.subtle"
	props.inset_shadow = if strong then "$kx.lip.inset_strong" else "$kx.lip.inset"
	return props
end

function M.Card(props: P): Node
	local p = table.clone(props)
	local level: Level = p.level or "raised"
	p.level = nil
	p.bg = p.bg or BG[level]
	p.rounded = p.rounded or "$kx.card.radius"
	if p.p == nil then
		p.p = "$space.4"
	end
	if level ~= "sunken" then
		M.lip(p)
	end
	return ui.column(p)
end

-- The glass that floats over the page.
function M.float_style(props: P): P
	props.rounded = props.rounded or "$radius.lg"
	props.bg = function()
		return if material.popups_see_through() then "$kx.float.tint" else "$kx.float.solid"
	end
	props.backdrop_blur = function()
		return if material.popups_see_through() then "$kx.float.blur" else 0
	end
	props.shadow = "$kx.float.shadow"
	M.lip(props, true)
	return props
end

function M.Float(props: P): Node
	local p = table.clone(props)
	if p.p == nil then
		p.p = "$space.1"
	end
	return ui.column(M.float_style(p))
end

-- A hairline that helps reading (between groups that need it), never a box.
function M.Rule(props: P?): Node
	local p = if props then table.clone(props) else {}
	p.h = 1
	p.w_full = true
	p.bg = p.bg or "$border.subtle"
	p.aria_hidden = true
	return ui.div(p)
end

return M
