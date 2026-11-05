--!strict
-- How surfaces stand: frosted (the window's glass, popovers over a blur)
-- or solid. One setting for the app (`material.frosted`, Settings ›
-- Appearance), overruled by the system: macOS's Reduce transparency, or
-- Windows' Transparency effects off, which the host reports
-- (src/transparency.rs), turn every glass solid.

local prefers = require("@kx/prefers")

local M = {}

M.frosted = ui.persist("frosted", true)

local ok, shared = pcall(ui.shared, "system.reduced_transparency", false)
local system_solid: State<boolean>? = if ok then shared :: any else nil

-- Whether popups can blur what's under them (not on Windows, where zgx's
-- popup windows have no backdrop yet: zgx-asks/popup-glass). Unknown, yes.
local ok_blur, popups = pcall(ui.shared, "system.popups_blur", true)
local popups_blur: State<boolean>? = if ok_blur then popups :: any else nil

-- Development: solid for the design PNGs (a capture can't hold the
-- system's backdrop), `KX_SOLID=1`.
local ok_dev, dev = pcall(ui.shared, "dev.solid", false)
local dev_solid = ok_dev and (dev :: any):peek() == true

-- Tracked: whether glass shows through.
function M.see_through(): boolean
	if dev_solid or not M.frosted:get() or prefers.reduced_transparency:get() then
		return false
	end
	return not (system_solid and system_solid:get())
end

-- Tracked: whether a popup's glass can blur.
function M.popups_see_through(): boolean
	return M.see_through() and (popups_blur == nil or popups_blur:get())
end

-- The window's backdrop follows: frosted, a blur of the desktop (Acrylic on
-- Windows); not, Mica's quiet tint under a solid ground.
ui.effect(function()
	local on = M.see_through()
	pcall(ui.invoke, "glass", on)
end)

-- The window's own ground: the theme's scrim over the system backdrop,
-- or, solid, the night (or dawn) itself.
function M.window_bg(): () -> string
	return function()
		return if M.see_through() then "$background" else "$kx.solid"
	end
end

return M
