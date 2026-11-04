--!strict
-- Keplorayx's icons (kx/icon_svgs, made by tools/icons.py): Dove's
-- library, refined, and Orbit's own glyphs, all on one 16-unit grid with
-- round caps and joins, drawn in the colour of the text around them.
--
-- The stroke is optical: a glyph keeps the same visual weight at every
-- size. At 16 points it is 1.25; larger, it thickens by the square root of
-- the scale rather than in proportion (a 24-point glyph would otherwise
-- look heavy), smaller, it thins as little, never under one point.
--
--   Icon { name = "orbit" }                 16 points, the text's colour
--   Icon { name = "inbox", size = 20, color = "$kx.attention" }
--   Icon { mark = "claude-mark", size = 14 } a brand or agent mark

local svgs = require("@kx/icon_svgs")

export type Props = {
	name: (string | () -> string)?,
	mark: string?,
	size: number?,
	color: string?,
	label: string?,
}

local BASE = 1.25

-- The stroke in the glyph's own units for a glyph drawn `size` across.
local function stroke_units(size: number): number
	local base = (ui.theme.get("kx.glyph.stroke") :: number?) or BASE
	local points = math.max(1, base * math.sqrt(size / 16))
	return points * 16 / size
end

local function glyph(name: string, size: number): string
	local src = svgs.glyphs[name] or svgs.glyphs["widget"]
	local units = string.format("%.3f", stroke_units(size))
	-- The root's width (the first); a part drawn with its own keeps it.
	local out = string.gsub(src, 'stroke%-width="1%.25"', `stroke-width="{units}"`, 1)
	return `code:{out}`
end

local M = {}

function M.has(name: string): boolean
	return svgs.glyphs[name] ~= nil
end

function M.names(): { string }
	local out = {}
	for name in svgs.glyphs do
		table.insert(out, name)
	end
	table.sort(out)
	return out
end

function M.mark_names(): { string }
	local out = {}
	for name in svgs.marks do
		table.insert(out, name)
	end
	table.sort(out)
	return out
end

function M.Icon(props: Props): Node
	local size = props.size or 16
	local src: any
	if props.mark then
		local code = svgs.marks[props.mark]
		src = if code then `code:{code}` else glyph("widget", size)
	else
		local name = props.name or "widget"
		src = if type(name) == "function" then function()
			return glyph(name(), size)
		end else glyph(name, size)
	end
	return ui.svg {
		src = src,
		w = size,
		h = size,
		shrink = 0,
		-- A glyph says nothing a screen reader needs unless it stands alone.
		aria_hidden = props.label == nil,
		label = props.label,
		tint = if props.mark then nil else (props.color or "currentColor"),
	}
end

return M
