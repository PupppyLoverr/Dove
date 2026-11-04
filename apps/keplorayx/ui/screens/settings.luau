--!strict
-- Settings: a page of sections down a quiet nav. Appearance, Agents,
-- Privacy and your account, Shortcuts. Every change applies as it's made.
-- The account is Keplorayx's own (Woozlit), optional and never on by
-- itself; agents' sign-ins are theirs and live under Agents.

local kx = require("@kx")
local store = require("@kx/store")

local M = {}

local section = ui.persist("settings.section", "appearance")
local accent = require("@kx/accent")
local scheme = ui.persist("settings.scheme", "system")

ui.effect(function()
	local s = scheme:get()
	if s == "system" then
		ui.theme.follow_system { dark = "orbit-night", light = "orbit-dawn" }
	else
		ui.theme.set(s)
	end
end)

local SECTIONS = {
	{ id = "appearance", title = "Appearance", icon = "palette-search" },
	{ id = "agents", title = "Agents", icon = "bot" },
	{ id = "privacy", title = "Privacy and account", icon = "local-only" },
	{ id = "shortcuts", title = "Shortcuts", icon = "keyboard" },
	{ id = "about", title = "About", icon = "info-circle" },
}

local function setting(title: string, hint: string?, control: Node): Node
	return ui.row {
		w_full = true,
		gap = "$space.4",
		items = "center",
		py = "$space.3",
		ui.column {
			grow = 1,
			min_w = 0,
			gap = "$kx.space.hair",
			kx.Label { title, literal = true },
			if hint then kx.Caption { hint, literal = true, wrap = true } else nil,
		},
		control,
	}
end

local function group(title: string, ...: Node): Node
	return ui.column {
		w_full = true,
		gap = "$space.1",
		kx.Heading { title, literal = true, pb = "$space.1" },
		ui.column {
			w_full = true,
			px = "$space.4",
			rounded = "$kx.card.radius",
			bg = "$surface.base",
			border = 1,
			border_color = "$border.subtle",
			...,
		},
	}
end

local function appearance(): Node
	return ui.column {
		w_full = true,
		gap = "$space.6",
		group(
			"Theme",
			setting(
				"Appearance",
				"Night and dawn, or follow the system.",
				kx.Segmented {
					value = scheme,
					label = "Appearance",
					width = 76,
					options = {
						{ label = "System", value = "system" },
						{ label = "Night", value = "orbit-night" },
						{ label = "Dawn", value = "orbit-dawn" },
					},
				}
			),
			kx.Rule {},
			setting(
				"Frosted glass",
				"The window shows a soft blur of what's behind it. Off, surfaces are solid.",
				kx.Switch { value = kx.material.frosted, label = "Frosted glass" }
			)
		),
		group(
			"Your star",
			setting("Accent", "The app's own light: the way on, focus, what's chosen.", (function(): Node
				local stars: { Node } = {}
				for _, s in accent.STARS do
					table.insert(stars, kx.Button {
						tone = "ghost",
						size = "sm",
						icon = true,
						label = s.name,
						tooltip = s.name,
						on_click = function()
							accent.chosen:set(s.id)
						end,
						ui.column {
							color = function()
								local kind = ui.theme.kind()
								local dark = not (kind == "light" or kind == "high-contrast-light")
								return ui.raw(if dark then s.night else s.dawn, "each star shows its own colour")
							end,
							opacity = function()
								return if accent.chosen:get() == s.id then 1 else 0.55
							end,
							kx.Icon { name = "star-four", size = 18 },
						},
					})
				end
				return ui.row { gap = "$kx.space.hair", table.unpack(stars) }
			end)()),
			kx.Rule {},
			setting("Setup", "Walk through First Light again.", kx.Button {
				tone = "secondary",
				size = "sm",
				on_click = function()
					ui.invoke("app.onboarding")
				end,
				"Open setup",
			})
		),
		group(
			"Motion",
			setting(
				"Reduce motion",
				"Moves land at once and fades stay short. Follows the system's setting too.",
				kx.Switch { value = kx.motion.reduce, label = "Reduce motion" }
			)
		),
	}
end

local function agent_row(a: store.Agent): Node
	return ui.row {
		w_full = true,
		gap = "$space.3",
		items = "center",
		py = "$space.3",
		kx.AgentMark { agent = a.id, size = 10 },
		ui.text { a.label, literal = true, grow = 1 },
		kx.Hint { "installed", literal = true },
	}
end

local function agents(): Node
	return ui.column {
		w_full = true,
		gap = "$space.6",
		group(
			"Installed",
			ui.show(
				function()
					return #store.agents:get() > 0
				end,
				ui.each(store.agents, agent_row, {
					key = function(a: store.Agent)
						return a.id
					end,
					w_full = true,
				}),
				kx.Empty {
					icon = "bot",
					title = "No agents found yet",
					hint = "Install one (Claude Code, Codex, Gemini CLI…) and it shows here. Keplorayx looks again at every launch.",
				}
			)
		),
		kx.Caption {
			"Each agent signs in with its own account, kept where its CLI keeps it. Keplorayx reads whether you're signed in and never sees the password.",
			literal = true,
			wrap = true,
		},
	}
end

local function privacy(): Node
	return ui.column {
		w_full = true,
		gap = "$space.6",
		group(
			"On this device",
			ui.row {
				w_full = true,
				gap = "$space.3",
				py = "$space.4",
				items = "start",
				ui.column { color = "$accent", pt = "$kx.space.hair", kx.Icon { name = "local-only", size = 20 } },
				ui.column {
					grow = 1,
					min_w = 0,
					gap = "$space.1",
					kx.Label { "Local only", literal = true },
					kx.Body {
						"Your projects, threads, settings and teammates are stored on this computer. Keplorayx sends none of it anywhere unless you sign in and turn on remote control.",
						literal = true,
						color = "$text.secondary",
					},
				},
			}
		),
		group(
			"What does leave",
			setting("The agents you run", "Each sends your prompts and the files it reads to its own model provider.", ui.div {}),
			kx.Rule {},
			setting("Services you connect", "MCP servers, Supermemory and cloud machines see what you send them.", ui.div {}),
			kx.Rule {},
			setting("Update checks", "Asks npm and GitHub whether your agents have new versions.", ui.div {})
		),
		group(
			"Keplorayx account",
			setting(
				"Sign in with Woozlit",
				"Optional. Lets your phone see and steer this computer's threads, and use it as a machine. Nothing syncs until you choose to.",
				kx.Button {
					tone = "secondary",
					size = "sm",
					disabled = true,
					tooltip = "Arrives with the account bridge",
					"Sign in",
				}
			)
		),
	}
end

local SHORTCUTS = {
	{ "New thread", "Ctrl N" },
	{ "Search and commands", "Ctrl K" },
	{ "Settings", "Ctrl ," },
	{ "Allow what an agent asks", "Y" },
	{ "Deny it", "N" },
	{ "Stop the agent", "Esc" },
}

local function shortcuts(): Node
	local rows: { Node } = {}
	for i, s in SHORTCUTS do
		if i > 1 then
			table.insert(rows, kx.Rule {})
		end
		local keys: { Node } = {}
		for key in string.gmatch(s[2], "%S+") do
			table.insert(keys, kx.Kbd { key })
		end
		table.insert(rows, setting(s[1], nil, ui.row { gap = "$space.1", table.unpack(keys) }))
	end
	return group("Keys", table.unpack(rows))
end

local function about(): Node
	return ui.column {
		w_full = true,
		gap = "$space.3",
		items = "start",
		ui.row {
			gap = "$space.3",
			items = "center",
			ui.column { color = "$accent", kx.Icon { name = "orbit", size = 32 } },
			ui.column {
				gap = "$kx.space.hair",
				kx.Heading { "Keplorayx", literal = true },
				kx.Caption { "on zgx · development build", literal = true },
			},
		},
		kx.Body {
			"Every agent is a body in orbit around your work: calm while it works, drawn closer when it needs you.",
			literal = true,
			color = "$text.secondary",
		},
	}
end

local PAGES: { [string]: () -> Node } = {
	appearance = appearance,
	agents = agents,
	privacy = privacy,
	shortcuts = shortcuts,
	about = about,
}

function M.view(): Node
	local items: { Node } = {}
	for _, s in SECTIONS do
		table.insert(items, kx.Row {
			size = "sm",
			selected = function()
				return section:get() == s.id
			end,
			on_click = function()
				section:set(s.id)
			end,
			ui.column { color = "$text.secondary", kx.Icon { name = s.icon, size = 16 } },
			ui.text { s.title, literal = true, grow = 1 },
		})
	end
	local pages: { [string]: Node } = {}
	for id, build in PAGES do
		pages[id] = ui.column { w_full = true, build() }
	end
	return ui.column {
		grow = 1,
		min_w = 0,
		h_full = true,
		ui.row {
			h = "$kx.titlebar",
			w_full = true,
			window_drag = true,
			items = "center",
			px = "$space.5",
			kx.Label { "Settings", literal = true },
		},
		ui.row {
			grow = 1,
			min_h = 0,
			w_full = true,
			ui.column { w = 220, shrink = 0, px = "$space.3", pt = "$space.2", gap = "$kx.space.hair", table.unpack(items) },
			ui.scroll {
				grow = 1,
				min_w = 0,
				h_full = true,
				ui.column {
					w_full = true,
					max_w = 640,
					px = "$space.6",
					pt = "$space.2",
					pb = "$space.8",
					ui.switch(section, pages),
				},
			},
		},
	}
end

return M
