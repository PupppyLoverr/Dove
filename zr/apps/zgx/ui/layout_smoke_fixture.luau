-- The six surfaces the theme check's stage 6 mounts, at the three widths in
-- src/layout_smoke.rs: a list row with its two lines, the band, a control, a
-- settings row with its description, the panel's card row, and an empty
-- state. Each is the shape the shell really uses — the row heights, line
-- heights and paddings are the theme's own tokens — so a theme whose body is
-- too large for its rows, or whose rows are too short for their two lines,
-- clips here exactly as it would in the window.
--
-- It is deliberately not the whole shell: the check runs before a theme is
-- installed, so it must not depend on anything the window has (a host, an
-- engine, a session) — only the design system and the tokens under test.
-- `themes.authored` is set by the host before this loads, so `ds.setup()`
-- wears the theme being checked.

local ds = require("@ds")
ds.setup()
-- (Worn here, before anything is mounted: a token that is not there when a
-- node is built is not there for that node.)
require("@ds/authored").wear()

local function words(text: string, size: string, line: string, weight: string?, color: string?): Node
	return ui.text {
		text,
		text_size = size,
		line_height = line,
		weight = weight,
		color = color or "$text.primary",
		truncate = true,
		max_w = 240,
	}
end

-- A list row as ds/session_row draws one: a title over its detail line, both
-- inside the row's two-line height (`M.TALL`, 46 — a fixed number there, as
-- here, so a theme's type has to fit inside it rather than stretch it).
local function row(id: string): Node
	return ui.row {
		id = id,
		w_full = true,
		h = ui.raw(46, "ds/session_row M.TALL: a session row with its detail line"),
		shrink = 0,
		pl = "$space.2",
		pr = "$space.2",
		gap = "$space.2",
		items = "center",
		ui.column {
			grow = 1,
			min_w = 0,
			gap = "$zeron.space.hair",
			words("Fix the failing test in the parser", "$font.size.md", "$font.line.md", "$font.weight.medium"),
			words("keplorayx · zgx/phase-a", "$font.size.xs", "$font.line.xs", "$font.weight.regular"),
		},
	}
end

-- The band every page's top strip draws: one line of type in it.
local function band(id: string): Node
	return ui.row {
		id = id,
		w_full = true,
		h = "$band.height",
		shrink = 0,
		pl = "$space.3",
		pr = "$space.3",
		items = "center",
		words("New chat", "$font.size.sm", "$font.line.sm", "$font.weight.medium", "$text.secondary"),
	}
end

-- A control: a button's height, one label in it.
local function control(id: string): Node
	return ui.row {
		id = id,
		h = "$density.control",
		shrink = 0,
		pl = "$space.3",
		pr = "$space.3",
		items = "center",
		rounded = "$radius.md",
		bg = "$surface.raised",
		words("Send", "$font.size.md", "$font.line.md", "$font.weight.medium"),
	}
end

-- A settings row: its name over its description, right-aligned control.
local function setting(id: string): Node
	return ui.row {
		id = id,
		w_full = true,
		min_h = "$density.row",
		shrink = 0,
		pl = "$space.3",
		pr = "$space.3",
		gap = "$space.4",
		items = "center",
		ui.column {
			grow = 1,
			min_w = 0,
			gap = "$zeron.space.hair",
			words("Interface size", "$font.size.md", "$font.line.md", "$font.weight.medium"),
			words("Menus, settings and conversations, scaled together", "$font.size.sm", "$font.line.sm", "$font.weight.regular", "$text.secondary"),
		},
		control("smoke-control-in-setting"),
	}
end

-- A panel row: a file's name and its counts on one line.
local function panel_row(id: string): Node
	return ui.row {
		id = id,
		w_full = true,
		h = "$density.row.compact",
		shrink = 0,
		pl = "$space.3",
		pr = "$space.3",
		gap = "$space.2",
		items = "center",
		words("screens/shell/sidebar.luau", "$font.size.xs", "$font.line.xs", "$font.weight.regular"),
		words("+12 −4", "$font.size.xxs", "$font.line.xxs", "$font.weight.medium", "$text.secondary"),
	}
end

local function frame(): Node
	return ui.column {
		w_full = true,
		h_full = true,
		gap = "$space.2",
		p = "$space.2",
		band("smoke-band"),
		ui.column {
			w_full = true,
			min_h = 0,
			gap = "$zeron.list.gap",
			row("smoke-row-1"),
			row("smoke-row-2"),
			row("smoke-row-3"),
			panel_row("smoke-panel-row"),
		},
		setting("smoke-setting"),
		ui.row {
			w_full = true,
			h = "$font.line.xxl",
			shrink = 0,
			items = "center",
			justify = "center",
			words("Interface size", "$font.size.xxl", "$font.line.xxl", "$font.weight.strong"),
		},
		(require("@ds/empty_state")) {
			size = "sm",
			icon = "git-branch",
			fill = true,
			title = "No changes",
			body = "The tree is clean: what an agent changes appears here as it saves.",
		},
	}
end

return frame()
