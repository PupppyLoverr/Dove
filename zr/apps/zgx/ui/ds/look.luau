-- The accent Zeron is drawn in: the theme's own, or one of Zeron's presets
-- (crates/theme AccentPreset), each the theme's (`zeron.accents`, lighter
-- on dark, deeper on light, as AccentPreset::color has them). One setting
-- for the whole app, kept across launches, as the material is
-- (ds/material); a window takes it through `theme` (`look.theme()`).

local material = require("@self/material")
local locale = require("@self/locale")

local M = {}

export type Accent = { value: string, label: string, preset: boolean? }

-- An accent, its name read as a string in the locale shown (`a.label`,
-- through its message as it's asked for: read in a function, it follows).
-- Zeron's own is its name in every language.
local function accent(value: string, preset: boolean?): Accent
	return setmetatable({ value = value, preset = preset }, {
		__index = function(_, field: string): string?
			if field == "label" then
				return if value == "zeron" then "Zeron" else locale.s(`ds.look.accent.{value}`)
			end
			return nil
		end,
	}) :: any
end

-- In Zeron's order: the theme's own, then the presets.
M.ACCENTS = {
	accent("default"),
	accent("zeron", true),
	accent("orange", true),
	accent("amber", true),
	accent("green", true),
	accent("cyan", true),
	accent("blue", true),
	accent("pink", true),
} :: { Accent }

M.accent = ui.persist("accent", "default")

function M.find(value: string): Accent
	for _, a in M.ACCENTS do
		if a.value == value then
			return a
		end
	end
	return M.ACCENTS[1]
end

-- An accent's ink, in whichever scheme draws it. The theme's own is its
-- own token (`zeron.accents.default`, the theme's `accent` as written), so
-- it keeps its colour inside a window that wears another.
function M.ink(a: Accent): string
	return `$zeron.accents.{a.value}`
end

-- The tokens a window overrides to wear `value` (the chosen accent by
-- default): its `accent`, and what's derived from it follows. Nothing for
-- the theme's own.
function M.theme(value: string?): { [string]: any }?
	local a = M.find(value or M.accent:get())
	if not a.preset then
		return nil
	end
	return { accent = `$zeron.accents.{a.value}` }
end

-- The interface's size: its body size in points, which is the theme's one
-- authored type token (`font.size`, 13 by default). Every step of the scale
-- is derived from it in ds/authored — `xxs` (a step under 11 at the default),
-- `xs` through `xxl`, a step at 16 (`lg2`), the line heights and the
-- conversation's prose — so this one number moves all of them, and nothing
-- re-scales what the theme already derived. Code's size has its own setting.
--
-- 11 to 18 is the theme check's clamp (src/authored.rs): below it a body
-- stops being readable, above it a row's second line is cut, and both are
-- things a person may still want at the ends of the range.
M.TEXT_SIZES = { 11, 12, 13, 14, 15, 16, 17, 18 }
M.text_size = ui.persist("text_size", 13)

-- How far each style moves the body text from the size chosen. (Declared
-- above `interface_size`, which reads it: a local declared further down is
-- not in that function's scope, so the name read there was a global that is
-- always nil and every call raised "attempt to index nil". The reader in
-- ds/tokens swallowed that in a `pcall`, so the window always wore the theme
-- file's own size and Settings' control moved only its own label.)
local STYLE_TEXT = { editorial = 2, dense = -1 }

-- The body size the window wears now: the person's choice, and the style's
-- own offset (Editorial sets a step larger).
function M.interface_size(): number
	return M.text_size:get() + (STYLE_TEXT[M.style:get()] or 0)
end

-- The interface's font and code's (Settings → Appearance, as the shipping
-- app's Interface font and Code font): a family installed on this computer,
-- by its name, or "" for the theme's own (Geist, Geist Mono). Code's sets
-- code blocks, the editor, diffs, tool output and terminals. Kept across
-- launches, for the main window.
M.interface_font = ui.persist("interface_font", "")
M.code_font = ui.persist("code_font", "")

-- The font tokens a window overrides for the families chosen (each a
-- family's name, "" for the theme's own; the chosen ones by default).
-- Nothing for the theme's own. (Quoted, so a name is read as a family
-- whatever it looks like, "3270 Nerd Font" too, never a number or a colour.)
function M.font_theme(interface: string?, code: string?): { [string]: any }?
	local sans = interface or M.interface_font:get()
	local mono = code or M.code_font:get()
	if sans == "" and mono == "" then
		return nil
	end
	local out: { [string]: any } = {}
	if sans ~= "" then
		out["font.sans"] = `"{sans}"`
	end
	if mono ~= "" then
		out["font.mono"] = `"{mono}"`
	end
	return out
end

-- Code's family by its name, for what takes a family rather than a token
-- (a span of inline code): the one chosen, else the theme's own.
function M.code_family(): string
	local chosen = M.code_font:get()
	if chosen ~= "" then
		return chosen
	end
	local own = ui.theme.get("font.mono")
	return if type(own) == "string" and own ~= "" then own else "mono"
end

-- (Keplorayx) The main window's style (Settings → Appearance): Zeron's own
-- look, or one of Keplorayx's, each a set of tokens the window wears over
-- its theme (its glass, cards, corners, text and selection), so every
-- screen follows at once.
--   glass      the window frosted through: thin cards, rounder corners
--   editorial  warm paper and ink in a serif, larger type, an amber accent
--   dense      a terminal's: near black, square, monospace, a green accent
M.STYLES = { "zeron", "glass", "editorial", "dense" }
M.style = ui.persist("kx_style", "zeron")

local function dark(): boolean
	local kind = ui.theme.kind()
	return not (kind == "light" or kind == "high-contrast-light")
end

-- The tokens a window wears for `value` (the chosen style by default),
-- nil for Zeron's own.
function M.style_theme(value: string?): { [string]: any }?
	local style = value or M.style:get()
	local d = dark()
	if style == "glass" then
		return {
			-- (The window's tint thinner, so the frost behind it shows.)
			background = if d then "#0b0b0d4d" else "#f5f5f75c",
			["zeron.inset.bg"] = if d then "alpha({text.primary}, 0.045)" else "#ffffff73",
			["zeron.inset.edge"] = if d then "#ffffff14" else "#ffffffb3",
			["zeron.inset.radius"] = 14,
			["radius.lg"] = 10,
			["radius.xl"] = 16,
			["selection.item"] = if d then "#ffffff1f" else "#ffffff99",
			["zeron.selected.flat.bg"] = if d then "#ffffff1f" else "#ffffff99",
		}
	elseif style == "editorial" then
		-- Warm paper and ink, set in a serif, the accent a warm amber.
		return {
			["font.sans"] = `"Georgia"`,
			background = if d then "#15120e" else "#f4efe6",
			["zeron.window"] = if d then "#15120e" else "#f4efe6",
			["zeron.inset.bg"] = if d then "#1c1814" else "#fbf8f2",
			["zeron.inset.edge"] = if d then "#f3e3c814" else "#3a2a1414",
			["text.primary"] = if d then "#ede5d8" else "#231c14",
			accent = if d then "#e0a55e" else "#a8641f",
			["accent.hover"] = if d then "#e8b373" else "#94571a",
			border = if d then "#f3e3c81a" else "#3a2a141a",
			["border.subtle"] = if d then "#f3e3c80f" else "#3a2a140f",
			["surface.overlay"] = if d then "#221d18f0" else "#fbf8f2f0",
			["zeron.inset.radius"] = 16,
			["radius.lg"] = 10,
			["radius.xl"] = 18,
			["zeron.list.gap"] = 4,
			["selection.item"] = "alpha({accent}, 0.18)",
			["zeron.selected.flat.bg"] = "alpha({accent}, 0.18)",
			hover = "alpha({accent}, 0.08)",
		}
	elseif style == "dense" then
		-- A terminal's: near black, square, monospace, a green accent.
		return {
			["font.sans"] = `"Geist Mono"`,
			background = if d then "#070908" else "#f7f8f7",
			["zeron.window"] = if d then "#070908" else "#f7f8f7",
			["zeron.inset.bg"] = if d then "#0c0f0d" else "#ffffff",
			["zeron.inset.edge"] = if d then "#7cf0b31f" else "#0d3b2526",
			["text.primary"] = if d then "#d7e4dc" else "#13201a",
			accent = if d then "#5fd3a0" else "#11845a",
			["accent.hover"] = if d then "#79deb1" else "#0e7550",
			border = if d then "#7cf0b31a" else "#0d3b251f",
			["border.subtle"] = if d then "#7cf0b30f" else "#0d3b2514",
			["surface.overlay"] = if d then "#0e1210f2" else "#fffffff2",
			["zeron.inset.radius"] = 4,
			["radius.sm"] = 2,
			["radius.md"] = 2,
			["radius.lg"] = 3,
			["radius.xl"] = 4,
			["zeron.list.gap"] = 0,
			["selection.item"] = "alpha({accent}, 0.16)",
			["zeron.selected.flat.bg"] = "alpha({accent}, 0.16)",
			hover = "alpha({accent}, 0.07)",
		}
	end
	return nil
end

-- What the main window wears over its theme (ds/palettes, worn by
-- ds/tokens): its style, the fonts chosen, and an accent chosen over the
-- theme's own. (The text size is not here: it is the theme's own authored
-- `font.size`, worn by ds/authored, so the whole scale derives from it
-- rather than being re-scaled after the fact.)
function M.window_theme(): { [string]: any }?
	local out: { [string]: any }? = nil
	for _, part in { M.style_theme(), M.font_theme(), M.theme() } do
		if part then
			out = out or {}
			for k, v in part do
				(out :: any)[k] = v
			end
		end
	end
	-- And its glass's tint where the desktop blurs less than macOS's does
	-- (ds/material).
	local glass = material.glass_theme()
	if glass then
		out = out or {}
		for k, v in glass do
			(out :: any)[k] = v
		end
	end
	return out
end

return M
