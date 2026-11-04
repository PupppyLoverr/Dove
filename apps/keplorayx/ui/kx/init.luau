--!strict
-- Keplorayx's design system, Orbit: `local kx = require("@kx")`. Screens
-- import from here and zgx's layout primitives alone; anything a second
-- screen would want moves in here, and into the gallery, before it's used.

local tokens = require("@kx/tokens")
local button = require("@kx/button")
local icon = require("@kx/icon")
local text = require("@kx/text")
local surface = require("@kx/surface")
local rows = require("@kx/rows")
local controls = require("@kx/controls")
local overlays = require("@kx/overlays")

return {
	setup = tokens.setup,
	tokens = tokens,
	motion = require("@kx/motion"),
	shape = require("@kx/shape"),
	material = require("@kx/material"),
	prefers = require("@kx/prefers"),
	icons = icon,

	Button = button.Button,
	Icon = icon.Icon,

	Title = text.Title,
	Heading = text.Heading,
	Body = text.Body,
	Label = text.Label,
	Caption = text.Caption,
	Hint = text.Hint,
	Mono = text.Mono,
	Numeric = text.Numeric,

	Card = surface.Card,
	Float = surface.Float,
	Rule = surface.Rule,
	float_style = surface.float_style,
	lip = surface.lip,

	Row = rows.Row,
	AgentMark = rows.AgentMark,
	Kbd = rows.Kbd,
	Empty = rows.Empty,

	Field = controls.Field,
	Switch = controls.Switch,
	Segmented = controls.Segmented,
	Pill = controls.Pill,

	Menu = overlays.Menu,
	MenuItem = overlays.MenuItem,
	Popover = overlays.Popover,
	Dialog = overlays.Dialog,
	Tooltip = overlays.Tooltip,
	toast = overlays.toast,
	Toasts = overlays.Toasts,
}
