--!strict
-- Rows and the small things that sit in them.
--
--   Row { selected = fn, on_click = …, leading, title, trailing }
--       a list's row: one wash for hover, press and selection, its corner
--       concentric with the list it sits in (`radius`, the list's inner)
--   AgentMark { agent = "codex", live = fn, needs_you = fn }
--       an agent's star in its own light; it breathes only while the agent
--       works, and turns warm only when it needs you
--   Kbd { "⌘K" }      a key
--   Empty { icon, title, hint, action }
--       an empty state: calm, one line of why, one way on

local motion = require("@kx/motion")
local tokens = require("@kx/tokens")
local icon = require("@kx/icon")
local text = require("@kx/text")

local M = {}

type P = { [any]: any }

local function read(v: any): any
	if type(v) == "function" then
		return v()
	elseif type(v) == "table" and v.get then
		return v:get()
	end
	return v
end

-- Zeron's row (ds/list_row, session_row; MIT): a compact row is 30 points,
-- a regular one 32, a two-line one 46; one wash, selection a firmer one.
local ROW = ui.variants {
	base = {
		w_full = true,
		items = "center",
		gap = "$space.2",
		px = "$space.2",
		py = 0,
		bg = "#00000000",
		cursor = "pointer",
		rounded = "$radius.lg",
		hover = { bg = "$hover" },
		active = { bg = "$pressed" },
		focus = { outline = 2, outline_color = "$focus.ring", outline_offset = -2 },
	},
	variants = {
		selected = { [true] = { bg = "$selection.item", hover = { bg = "$selection.item" } }, [false] = {} },
		size = { sm = { h = 30 }, md = { h = 32 }, lg = { h = 46, py = "$space.1" } },
	},
	defaults = { selected = false, size = "md" },
}

-- Each agent's mark in the icon library (its own glyph, in its light).
M.MARKS = {
	claude_code = "claude-mark",
	codex = "openai-mark",
	cursor = "cursor-mark",
	devin = "devin-mark",
	grok = "grok-mark",
	hermes = "hermes-mark",
	open_code = "opencode-mark",
	pi = "pi-mark",
	antigravity = "antigravity-mark",
	copilot = "brand-github",
	kimi = "k22-mark",
} :: { [string]: string }

function M.Row(props: P): Node
	local p = table.clone(props)
	local selected = p.selected
	p.variants = ROW {
		selected = if selected == nil then nil else function()
			return read(selected) == true
		end,
		size = p.size,
	}
	p.selected, p.size = selected, nil
	if p.radius then
		p.rounded = p.radius
		p.radius = nil
	end
	return ui.button(p)
end

-- An agent's mark: its own glyph in its light (a star for an agent without
-- one). `size` is the glyph's; `dot = true` draws the small round light
-- alone (Constellation, a chip). Live, it pulses on the engine's clock (no
-- script work); needing you, it turns warm.
function M.AgentMark(props: P): Node
	local size = props.size or 14
	local colour = tokens.agent(props.agent)
	local live, needs = props.live, props.needs_you
	local function warm(): boolean
		return needs ~= nil and read(needs) == true
	end
	local body: P
	if props.dot then
		body = {
			w = size,
			h = size,
			shrink = 0,
			rounded = "$radius.full",
			aria_hidden = true,
			bg = function()
				return if warm() then "$kx.attention" else colour
			end,
		}
	else
		body = {
			w = size,
			h = size,
			shrink = 0,
			items = "center",
			justify = "center",
			aria_hidden = true,
			color = function()
				return if warm() then "$kx.attention" else colour
			end,
			-- A brand's mark is drawn as itself (`mark`), Orbit's star as a glyph.
			if M.MARKS[props.agent or ""]
				then icon.Icon { mark = M.MARKS[props.agent or ""], size = size, color = "currentColor" }
				else icon.Icon { name = "star-four", size = size },
		}
	end
	if live == nil then
		return ui.column(body)
	end
	-- Live: the pulse runs only while it shows (motion.looping), so a
	-- resting row costs nothing.
	return ui.switch(function(): string
		return if read(live) and not warm() then "live" else "still"
	end, {
		live = motion.pulse(table.clone(body), 0.45, ui.column),
		still = ui.column(body),
	})
end

function M.Kbd(props: P): Node
	local p = table.clone(props)
	return ui.row {
		h = 18,
		px = "$space.1",
		min_w = 18,
		items = "center",
		justify = "center",
		rounded = "$radius.sm",
		bg = "$surface.base",
		border = 1,
		border_color = "$border.subtle",
		ui.text {
			p[1],
			literal = true,
			font = "$font.sans",
			text_size = "$font.size.xs",
			weight = "$font.weight.medium",
			color = "$text.secondary",
		},
	}
end

function M.Empty(props: P): Node
	return ui.column {
		w_full = true,
		items = "center",
		justify = "center",
		gap = "$space.2",
		py = "$space.7",
		px = "$space.5",
		if props.icon then ui.column {
			mb = "$space.1",
			color = "$text.tertiary",
			icon.Icon { name = props.icon, size = 24 },
		} else nil,
		text.Label { props.title, literal = true, align = "center" },
		if props.hint then text.Caption { props.hint, literal = true, align = "center", wrap = true, max_w = 320 } else nil,
		if props.action then ui.row { mt = "$space.2", props.action } else nil,
	}
end

return M
