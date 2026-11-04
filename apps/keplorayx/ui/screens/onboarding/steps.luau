--!strict
-- First Light's steps, in order: each a title, whether it can be skipped,
-- what its button says, and its view. Copy is short, warm, plain.

local kx = require("@kx")
local store = require("@kx/store")
local accent = require("@kx/accent")
local ray = require("@screens/ray")

local M = {}

export type Step = { id: string, title: string, optional: boolean?, next: string?, view: () -> Node }

local function heading(title: string, line: string): Node
	return ui.column {
		w_full = true,
		gap = "$space.2",
		kx.Title { title, literal = true },
		kx.Body { line, literal = true, color = "$text.secondary" },
	}
end

-- ── Welcome ────────────────────────────────────────────────────────────

M.name = ui.persist("onboarding.name", "")
local scheme = ui.persist("settings.scheme", "system")

local function star_button(s: accent.Star): Node
	return ui.button {
		w = 44,
		h = 44,
		p = 0,
		rounded = "$radius.full",
		bg = "#00000000",
		hover = { bg = "$hover" },
		items = "center",
		justify = "center",
		cursor = "pointer",
		label = s.name,
		tooltip = s.name,
		role = "radio",
		checked = function()
			return accent.chosen:get() == s.id
		end,
		outline = function()
			return if accent.chosen:get() == s.id then 2 else 0
		end,
		outline_color = "$accent",
		on_click = function()
			accent.chosen:set(s.id)
		end,
		ui.column {
			color = function()
				local kind = ui.theme.kind()
				local dark = not (kind == "light" or kind == "high-contrast-light")
				return ui.raw(if dark then s.night else s.dawn, "each star shows its own colour")
			end,
			kx.Icon { name = "star-four", size = 22 },
		},
	}
end

local function welcome(): Node
	local stars: { Node } = {}
	for _, s in accent.STARS do
		table.insert(stars, star_button(s))
	end
	return ui.column {
		w_full = true,
		gap = "$space.5",
		heading("Welcome to Keplorayx", "Every agent is a body in orbit around your work. Let's set up your sky."),
		ui.column {
			w_full = true,
			gap = "$space.2",
			kx.Label { "What should we call you?", literal = true },
			kx.Field { value = M.name, placeholder = "Your name", label = "Your name", w = 320 },
		},
		ui.row {
			w_full = true,
			gap = "$space.6",
			ui.column {
				gap = "$space.2",
				kx.Label { "Light", literal = true },
				kx.Segmented {
					value = scheme,
					label = "Light",
					width = 72,
					options = {
						{ label = "System", value = "system" },
						{ label = "Night", value = "orbit-night" },
						{ label = "Dawn", value = "orbit-dawn" },
					},
				},
			},
			ui.column {
				gap = "$space.1",
				kx.Label { "Your star", literal = true },
				ui.row { gap = "$kx.space.hair", role = "radiogroup", label = "Your star", table.unpack(stars) },
			},
		},
	}
end

ui.effect(function()
	local s = scheme:get()
	if s == "system" then
		ui.theme.follow_system { dark = "orbit-night", light = "orbit-dawn" }
	else
		ui.theme.set(s)
	end
end)

-- ── Your agents ────────────────────────────────────────────────────────

-- The agents setup offers, with how to install each that isn't yet.
local CATALOG = {
	{ id = "claude_code", label = "Claude Code", install = "npm i -g @anthropic-ai/claude-code" },
	{ id = "codex", label = "Codex", install = "npm i -g @openai/codex" },
	{ id = "gemini", label = "Gemini CLI", install = "npm i -g @google/gemini-cli" },
	{ id = "open_code", label = "OpenCode", install = "npm i -g opencode-ai" },
	{ id = "copilot", label = "Copilot", install = "npm i -g @github/copilot" },
	{ id = "qwen", label = "Qwen Code", install = "npm i -g @qwen-code/qwen-code" },
}

local function installed(id: string): boolean
	for _, a in store.agents:get() do
		if a.id == id then
			return true
		end
	end
	return false
end

local function agent_row(a: { id: string, label: string, install: string }): Node
	return ui.row {
		w_full = true,
		h = 44,
		gap = "$space.3",
		items = "center",
		px = "$space.3",
		kx.AgentMark {
			agent = a.id,
			size = 10,
			live = function()
				return installed(a.id)
			end,
		},
		ui.text { a.label, literal = true, grow = 1, weight = "$font.weight.medium" },
		ui.show(
			function()
				return installed(a.id)
			end,
			kx.Hint { "Ready", literal = true, color = "$success" },
			ui.row {
				gap = "$space.2",
				items = "center",
				ui.text { a.install, literal = true, font = "$font.mono", text_size = "$font.size.xs", color = "$text.tertiary" },
				kx.Button {
					tone = "secondary",
					size = "xs",
					on_click = function()
						ui.clipboard.write(a.install)
						kx.toast { title = "Copied", body = "Paste it into a terminal" }
					end,
					"Copy",
				},
			}
		),
	}
end

local function agents(): Node
	local rows: { Node } = {}
	for _, a in CATALOG do
		table.insert(rows, agent_row(a))
	end
	return ui.column {
		w_full = true,
		gap = "$space.4",
		heading("Your agents", "Keplorayx runs the agents you already use. Each keeps its own sign-in."),
		ui.column {
			w_full = true,
			py = "$space.1",
			rounded = "$kx.card.radius",
			bg = "$surface.base",
			border = 1,
			border_color = "$border.subtle",
			table.unpack(rows),
		},
	}
end

-- ── Your first project ─────────────────────────────────────────────────

local function project(): Node
	local chosen = ui.computed(function()
		local list = store.projects:get()
		return list[#list]
	end)
	return ui.column {
		w_full = true,
		gap = "$space.5",
		heading("Your first project", "Choose a folder to work in. Your agents run there, on this computer."),
		ui.show(
			function()
				return chosen:get() ~= nil
			end,
			kx.Card {
				w_full = true,
				gap = "$space.1",
				ui.row {
					gap = "$space.3",
					items = "center",
					ui.column { color = "$accent", kx.Icon { name = "folder", size = 24 } },
					ui.column {
						grow = 1,
						min_w = 0,
						gap = "$kx.space.hair",
						ui.text {
							function()
								local p = chosen:get()
								return if p then p.name else ""
							end,
							literal = true,
							weight = "$font.weight.semibold",
							text_size = "$font.size.lg",
						},
						ui.text {
							function()
								local p = chosen:get()
								return if p then p.path else ""
							end,
							literal = true,
							truncate = true,
							font = "$font.mono",
							text_size = "$font.size.xs",
							color = "$text.tertiary",
						},
					},
					kx.Button { tone = "ghost", size = "sm", on_click = ray.pick_folder, "Change" },
				},
			},
			ui.row {
				gap = "$space.3",
				items = "center",
				kx.Button { tone = "secondary", size = "lg", on_click = ray.pick_folder, kx.Icon { name = "folder-add", size = 18 }, "Choose a folder" },
				kx.Hint { "or drop one here", literal = true },
			}
		),
	}
end

-- ── Your team (optional) ───────────────────────────────────────────────

M.teammate = ui.persist("onboarding.teammate", "")
M.role = ui.persist("onboarding.role", "reviewer")

local ROLES = {
	reviewer = "reads every change before it lands",
	tester = "keeps the tests honest",
	writer = "turns what changed into words",
}

local function team(): Node
	return ui.column {
		w_full = true,
		gap = "$space.4",
		heading("Your team", "Add a teammate: an agent with a name, a role and a voice of its own. You can do this later."),
		ui.row {
			w_full = true,
			gap = "$space.4",
			ui.column {
				gap = "$space.2",
				kx.Label { "Name", literal = true },
				kx.Field { value = M.teammate, placeholder = "Ada", label = "Teammate's name", w = 200 },
			},
			ui.column {
				gap = "$space.2",
				kx.Label { "Role", literal = true },
				kx.Segmented {
					value = M.role,
					label = "Role",
					width = 84,
					options = {
						{ label = "Reviewer", value = "reviewer" },
						{ label = "Tester", value = "tester" },
						{ label = "Writer", value = "writer" },
					},
				},
			},
		},
		kx.Card {
			w_full = true,
			gap = "$space.2",
			ui.row {
				gap = "$space.2",
				items = "center",
				kx.AgentMark { agent = "claude_code", size = 8 },
				ui.text {
					function()
						local n = M.teammate:get()
						return if n == "" then "Ada" else n
					end,
					literal = true,
					weight = "$font.weight.semibold",
				},
				kx.Hint { "introduces themselves", literal = true },
			},
			kx.Body {
				function()
					local n = M.teammate:get()
					local role = M.role:get()
					return `Hi, I'm {if n == "" then "Ada" else n}, your {role}. I {ROLES[role] or "help"}, and I'll ask before I change anything.`
				end,
				literal = true,
				color = "$text.secondary",
			},
		},
	}
end

-- ── The notch and shortcuts ────────────────────────────────────────────

local function notch(): Node
	local glow = kx.motion.breath(0.5, 1)
	local function key_row(label: string, keys: { string }): Node
		local caps: { Node } = {}
		for _, k in keys do
			table.insert(caps, kx.Kbd { k })
		end
		return ui.row {
			w_full = true,
			h = 32,
			items = "center",
			gap = "$space.2",
			ui.text { label, literal = true, grow = 1, color = "$text.secondary" },
			table.unpack(caps),
		}
	end
	return ui.column {
		w_full = true,
		gap = "$space.4",
		heading("The notch, and a few keys", "Your agents live at the top of the screen too. It glows when one needs you."),
		-- The island, previewed: a dark capsule, the lip of light breathing.
		ui.row {
			w_full = true,
			justify = "center",
			ui.row {
				h = 36,
				px = "$space.4",
				gap = "$space.2",
				items = "center",
				rounded = "$radius.full",
				bg = "$kx.sky.deep",
				border = 1,
				border_color = "$border.subtle",
				inset_shadow = "$kx.lip.inset_strong",
				shadow = "$shadow.md",
				kx.AgentMark { agent = "claude_code", size = 7, live = function()
					return true
				end },
				kx.AgentMark { agent = "codex", size = 7, needs_you = function()
					return true
				end },
				ui.text { "Codex needs you", literal = true, text_size = "$font.size.sm", color = "$kx.attention", opacity = glow },
			},
		},
		ui.column {
			w_full = true,
			px = "$space.3",
			rounded = "$kx.card.radius",
			bg = "$surface.base",
			border = 1,
			border_color = "$border.subtle",
			key_row("Search, commands, anything", { "Ctrl", "K" }),
			key_row("A new thread", { "Ctrl", "N" }),
			key_row("What changed", { "Ctrl", "J" }),
			key_row("Allow or deny what an agent asks", { "Y", "N" }),
		},
	}
end

-- ── Privacy ────────────────────────────────────────────────────────────

local function privacy(): Node
	return ui.column {
		w_full = true,
		gap = "$space.4",
		heading("Private by default", "Everything you make here stays on this computer."),
		ui.column {
			w_full = true,
			gap = "$space.3",
			p = "$space.4",
			rounded = "$kx.card.radius",
			bg = "$surface.base",
			border = 1,
			border_color = "$border.subtle",
			ui.row {
				gap = "$space.3",
				items = "start",
				ui.column { color = "$accent", kx.Icon { name = "local-only", size = 20 } },
				kx.Body {
					"Projects, threads, settings and teammates live on this device. Keplorayx doesn't send them anywhere.",
					literal = true,
				},
			},
			ui.row {
				gap = "$space.3",
				items = "start",
				ui.column { color = "$text.tertiary", kx.Icon { name = "bot", size = 20 } },
				kx.Body {
					"Your agents send your prompts, and the files they read, to the model you chose for them.",
					literal = true,
					color = "$text.secondary",
				},
			},
		},
		ui.row {
			w_full = true,
			gap = "$space.3",
			items = "center",
			ui.column {
				grow = 1,
				min_w = 0,
				gap = "$kx.space.hair",
				kx.Label { "A Keplorayx account is optional", literal = true },
				kx.Caption {
					"Sign in with Woozlit later to reach this computer from your phone. Nothing syncs until you choose to.",
					literal = true,
					wrap = true,
				},
			},
		},
	}
end

M.ORDER = {
	{ id = "welcome", title = "Welcome", view = welcome },
	{ id = "agents", title = "Your agents", view = agents },
	{ id = "project", title = "Your first project", view = project },
	{ id = "team", title = "Your team", optional = true, view = team },
	{ id = "notch", title = "The notch and shortcuts", view = notch },
	{ id = "privacy", title = "Privacy", next = "Start", view = privacy },
} :: { Step }

return M
