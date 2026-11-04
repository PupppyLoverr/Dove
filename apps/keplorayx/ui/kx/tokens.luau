--!strict
-- Orbit's theme layer: the two themes (themes/*.json, which restyle in
-- place as they're saved) and Keplorayx's own tokens, each declared with a
-- default in standard terms so any theme (an imported VS Code theme
-- included) styles the app. Values live here and in the theme files;
-- screens name tokens and nothing else.

local shape = require("@kx/shape")

local M = {}

-- Each agent's own light: it follows the agent everywhere (the sidebar,
-- the notch, its turns, its diffs). Luminous and cool, never the warm
-- attention colour, so "needs you" is never mistaken for an agent.
M.AGENTS = {
	claude_code = "#c9a8ff",
	codex = "#6fd8c6",
	cursor = "#dfe5f5",
	devin = "#8fc2ff",
	grok = "#b9c2d6",
	gemini = "#7ea6ff",
	open_code = "#9fe38e",
	pi = "#f0a6d8",
	antigravity = "#a99cff",
	copilot = "#8fdcff",
	amp = "#ff9fb0",
	droid = "#c4e07a",
	goose = "#e8d48a",
	qwen = "#b59bff",
	kilo = "#79e0b0",
	cline = "#9ec9ff",
	hermes = "#d6b8ff",
	kimi = "#8ad0f0",
	shell = "#aab4cc",
}

local ready = false

function M.setup()
	if ready then
		return
	end
	ready = true
	local agents: { [string]: string } = {}
	for id, colour in M.AGENTS do
		agents[id] = colour
	end
	ui.theme.tokens {
		kx = {
			-- Half-steps of the spacing scale (space.1–8: 4, 8, 12, 16, 24,
			-- 32, 48, 64), for controls only.
			space = { hair = 2, tight = 6, snug = 10 },
			attention = "{warning}",
			["attention.soft"] = "alpha({warning}, 0.16)",
			danger = {
				soft = "alpha({danger}, 0.14)",
				hover = "alpha({danger}, 0.22)",
				active = "alpha({danger}, 0.3)",
			},
			-- An agent's or the accent's light laid softly behind something.
			["accent.soft"] = "alpha({accent}, 0.14)",
			glyph = { stroke = 1.25 },
			["lip"] = "alpha({text.primary}, 0.12)",
			["lip.strong"] = "alpha({text.primary}, 0.22)",
			["lip.inset"] = "0 1 0 0 {kx.lip}",
			["lip.inset_strong"] = "0 1 0 0 {kx.lip.strong}",
			sky = { deep = "{background}", mid = "{surface.sunken}", glow = "{accent}" },
			agent = agents,
			-- A diff's added and removed lines: their colours, washed.
			diff = { add = "alpha({success}, 0.12)", del = "alpha({danger}, 0.12)" },
			-- The soft light round a body that's live, or needs you.
			glow = {
				live = "0 0 10 0 alpha({accent}, 0.45)",
				attention = "0 0 14 1 alpha({warning}, 0.55)",
			},
			-- What veils the window under a drop.
			veil = "alpha({background}, 0.86)",
			-- A switch's knob: the brightest thing on any ground.
			knob = "#ffffff",
			-- The window's ground when it isn't glass.
			solid = "{surface.sunken}",
			-- The reading measure: about 68 characters of the body face.
			measure = 680,
			-- The window's own geometry.
			titlebar = 44,
			sidebar = 272,
			-- The display face, for headings and the wordmark.
			display = "{font.sans}",
			-- Numbers that change in place (timers, costs): the UI face; the
			-- figures are kept tabular where they're drawn.
			numeric = "{font.sans}",
			-- The glass that floats: menus, popovers, the Ray bar's results.
			float = {
				tint = "alpha({surface.overlay}, 0.86)",
				solid = "{popover.bg}",
				blur = 32,
				shadow = "{shadow.lg}",
			},
			-- The pill: the Ray bar and the thread's composer.
			pill = { radius = "{radius.xl}", inset = 6 },
			card = { radius = "{radius.xl}", inset = "{space.2}" },
			-- Syntax, in roles of the theme's own colours.
			syntax = {
				keyword = "{accent}",
				string = "{success}",
				number = "{warning}",
				comment = "{text.tertiary}",
				["function"] = "{info}",
				type = "{accent}",
				punctuation = "{text.secondary}",
				inserted = "{success}",
				deleted = "{danger}",
			},
		},
	}
	ui.theme.load("kx/themes/orbit-night.json")
	ui.theme.load("kx/themes/orbit-dawn.json")
	ui.theme.follow_system { dark = "orbit-night", light = "orbit-dawn" }

	-- Corners made from others, made again as the theme changes: inner =
	-- outer − inset, always.
	ui.effect(function()
		ui.theme.tokens {
			kx = {
				pill = { inner = shape.nest("$kx.pill.radius", 1, "$kx.pill.inset") },
				card = { inner = shape.nest("$kx.card.radius", 1, "$kx.card.inset") },
				menu = { row = shape.nest("$radius.lg", 1, "$space.1") },
				segmented = { puck = shape.nest("$radius.md", "$kx.space.hair") },
			},
		}
	end)
end

-- The token for an agent's colour (`$kx.agent.codex`), any id.
function M.agent(id: string?): string
	if id and M.AGENTS[id] then
		return `$kx.agent.{id}`
	end
	return "$kx.agent.shell"
end

return M
