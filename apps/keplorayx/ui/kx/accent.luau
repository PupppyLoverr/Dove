--!strict
-- Your accent: one of a few stars, chosen at setup or in Settings. It
-- restyles the app's own light (the way on, focus, selection) through a
-- theme scope on each window's root, so it follows night and dawn alike.
--
--   ui.row { theme = accent.scope(), … }      a window's root
--   accent.STARS                               the choices, in order

local M = {}

export type Star = { id: string, name: string, night: string, dawn: string }

M.STARS = {
	{ id = "kepler", name = "Kepler", night = "#86a8ff", dawn = "#3d63e0" },
	{ id = "vega", name = "Vega", night = "#9fd8ff", dawn = "#0f7fb8" },
	{ id = "lyra", name = "Lyra", night = "#c3a6ff", dawn = "#6b46d6" },
	{ id = "aurora", name = "Aurora", night = "#7fe0c2", dawn = "#11896a" },
	{ id = "rigel", name = "Rigel", night = "#f2a6c8", dawn = "#c03f7e" },
} :: { Star }

M.chosen = ui.persist("kx.accent", "kepler")

function M.star(id: string?): Star
	for _, s in M.STARS do
		if s.id == id then
			return s
		end
	end
	return M.STARS[1]
end

-- The theme scope for a window's root: nil for the theme's own accent.
function M.scope(): () -> any
	return function()
		local id = M.chosen:get()
		if id == "kepler" then
			-- The theme as it is (a nil scope would be zgx's own).
			return ui.theme.current:get()
		end
		local s = M.star(id)
		local kind = ui.theme.kind()
		local dark = not (kind == "light" or kind == "high-contrast-light")
		local colour = if dark then s.night else s.dawn
		-- Over the theme in use: a scope without one starts from zgx's own.
		return { theme = ui.theme.current:get(), accent = colour, ["accent.hover"] = colour, info = colour }
	end
end

return M
