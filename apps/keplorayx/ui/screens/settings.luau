--!strict
-- Settings, laid out as Zeron's are (screens/settings; MIT, see
-- THIRD_PARTY_NOTICES.md): a quiet list of pages, and each page's groups as
-- lifted cards of rows, a name and a line of why on the left, the control
-- on the right. Every change applies as it's made. The account is
-- Keplorayx's own (Woozlit), optional and never on by itself; agents'
-- sign-ins are theirs and live under Agents.

local kx = require("@kx")
local store = require("@kx/store")

local M = {}

local section = ui.persist("settings.section", "general")
local accent = require("@kx/accent")
local scheme = ui.persist("settings.scheme", "system")

ui.effect(function()
	local s = scheme:get()
	-- Development: the theme the host was asked for wins (tools/shot.ps1).
	local dev = (ui.shared("dev.theme", "") :: any):get() :: string
	if dev ~= "" then
		ui.theme.set(dev)
	elseif s == "system" then
		ui.theme.follow_system { dark = "orbit-night", light = "orbit-dawn" }
	else
		ui.theme.set(s)
	end
end)

local SECTIONS = {
	{ id = "general", title = "General", icon = "settings-minimalistic" },
	{ id = "appearance", title = "Appearance", icon = "palette-search" },
	{ id = "agents", title = "Agents", icon = "bot" },
	{ id = "notifications", title = "Notifications", icon = "bell" },
	{ id = "shortcuts", title = "Shortcuts", icon = "keyboard" },
	{ id = "account", title = "Account", icon = "user" },
	{ id = "data", title = "Privacy and data", icon = "local-only" },
	{ id = "about", title = "About", icon = "info-circle" },
}

local function setting(title: string, hint: string?, control: Node?): Node
	return ui.row {
		w_full = true,
		gap = "$space.4",
		items = "center",
		py = "$space.3",
		min_h = 52,
		ui.column {
			grow = 1,
			min_w = 0,
			gap = "$kx.space.hair",
			ui.text { title, literal = true, text_size = "$font.size.md" },
			if hint then kx.Caption { hint, literal = true, wrap = true } else nil,
		},
		control,
	}
end

local function group(title: string?, ...: Node): Node
	local rows: { Node } = {}
	for i, r in { ... } do
		if i > 1 then
			table.insert(rows, kx.Rule {})
		end
		table.insert(rows, r)
	end
	return ui.column {
		w_full = true,
		gap = "$space.2",
		if title then ui.text { title, literal = true, text_size = "$font.size.sm", color = "$text.tertiary", px = "$space.1" } else nil,
		ui.column {
			w_full = true,
			px = "$space.4",
			rounded = "$radius.xl",
			bg = "$surface.base",
			inset_shadow = "$kx.lip.inset",
			table.unpack(rows),
		},
	}
end

local function page(title: string, ...: Node): Node
	return ui.column {
		w_full = true,
		gap = "$space.6",
		ui.text { title, literal = true, text_size = "$font.size.xl", weight = "$font.weight.medium" },
		...,
	}
end

-- A setting Dove's store keeps (read once, written as it changes).
local stored = ui.state("settings.stored", {} :: { [string]: string })
local function load_stored()
	store.settings():after(function(all: any)
		stored:set(all or {})
	end, function() end)
end
local function stored_flag(key: string, default: boolean): State<boolean>
	local s = ui.state(`settings.flag.{key}`, default)
	ui.effect(function()
		local v = stored:get()[key]
		if v ~= nil then
			s:set(v ~= "false" and v ~= "0")
		end
	end)
	return s
end

local function general(): Node
	local name = ui.persist("kx.profile.name", "")
	local keys = stored_flag("kx.global_keys", true)
	return page(
		"General",
		group(
			nil,
			setting("Your name", "What Keplorayx calls you, here and in your teammates' briefs.", kx.Field {
				value = name,
				placeholder = "Your name",
				label = "Your name",
				w = 220,
			}),
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
			"From anywhere",
			setting(
				"Keys from any app",
				"Ctrl+Alt+Space brings Keplorayx forward to ask; Ctrl+Alt+A and Ctrl+Alt+D allow or deny what an agent waits on. Applies at the next launch.",
				kx.Switch {
					value = keys,
					label = "Keys from any app",
					on_change = function(on: boolean)
						store.set_setting("kx.global_keys", on)
					end,
				}
			),
			setting("In the tray", "Keplorayx keeps an icon by the clock: click it to open, or use its menu.", nil)
		)
	)
end

local function appearance(): Node
	return page(
		"Appearance",
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
			setting(
				"Frosted glass",
				"The window shows a soft blur of what's behind it. Off, surfaces are solid.",
				kx.Switch { value = kx.material.frosted, label = "Frosted glass" }
			)
		),
		group(
			"Your star",
			setting("Accent", "The app's own light: focus, what's chosen, your mark.", (function(): Node
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
								return if accent.chosen:get() == s.id then 1 else 0.45
							end,
							kx.Icon { name = "star-four", size = 18 },
						},
					})
				end
				return ui.row { gap = "$kx.space.hair", table.unpack(stars) }
			end)())
		),
		group(
			"Motion",
			setting(
				"Reduce motion",
				"Moves land at once and fades stay short. Follows the system's setting too.",
				kx.Switch { value = kx.motion.reduce, label = "Reduce motion" }
			)
		)
	)
end

local function agent_row(a: store.Agent): Node
	return ui.row {
		w_full = true,
		gap = "$space.3",
		items = "center",
		h = 44,
		kx.AgentMark { agent = a.id, size = 16 },
		ui.text { a.label, literal = true, grow = 1 },
		kx.StatusIcon { kind = "done", size = 14 },
		kx.Hint { "On this device", literal = true },
	}
end

local function agents(): Node
	local ray = require("@screens/ray")
	return page(
		"Agents",
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
		group(
			"New threads",
			setting(
				"What an agent may do before it asks",
				"Ask first is the safest; Full access runs without asking, best in a worktree.",
				kx.Segmented {
					value = ray.access,
					label = "Access",
					width = 84,
					options = {
						{ label = "Ask first", value = "ask" },
						{ label = "Edit files", value = "edits" },
						{ label = "Full", value = "full" },
					},
				}
			)
		),
		kx.Caption {
			"Each agent signs in with its own account, kept where its CLI keeps it. Keplorayx reads whether you're signed in and never sees the password.",
			literal = true,
			wrap = true,
		}
	)
end

local function notifications(): Node
	return page(
		"Notifications",
		group(
			nil,
			setting(
				"When an agent needs you",
				"A word from the system while Keplorayx is in the background.",
				kx.Switch { value = ui.persist("kx.notify.needs", true), label = "When an agent needs you" }
			),
			setting(
				"When a thread finishes",
				"Only while you're away from the window.",
				kx.Switch { value = ui.persist("kx.notify.done", true), label = "When a thread finishes" }
			)
		),
		kx.Caption { "The taskbar's badge counts the threads waiting on you.", literal = true, wrap = true }
	)
end

local SHORTCUTS = {
	{ "New thread", "Ctrl N" },
	{ "Search and commands", "Ctrl K" },
	{ "Show or hide the sidebar", "Ctrl B" },
	{ "Changes", "Ctrl J" },
	{ "Files", "Ctrl P" },
	{ "Browser", "Ctrl Shift B" },
	{ "Terminal", "Ctrl `" },
	{ "Inbox", "Ctrl I" },
	{ "Team", "Ctrl Shift T" },
	{ "Constellation", "Ctrl −" },
	{ "Settings", "Ctrl ," },
	{ "Allow what an agent asks", "Y" },
	{ "Deny it", "N" },
}

local GLOBAL = {
	{ "Ask, from any app", "Ctrl Alt Space" },
	{ "Allow, from any app", "Ctrl Alt A" },
	{ "Deny, from any app", "Ctrl Alt D" },
	{ "Speak instead of typing", "Win H" },
}

local function keys_of(list: { { string } }): { Node }
	local rows: { Node } = {}
	for _, s in list do
		local keys: { Node } = {}
		for key in string.gmatch(s[2], "%S+") do
			table.insert(keys, kx.Kbd { key })
		end
		table.insert(rows, setting(s[1], nil, ui.row { gap = "$space.1", table.unpack(keys) }))
	end
	return rows
end

local function shortcuts(): Node
	return page("Shortcuts", group("In the window", table.unpack(keys_of(SHORTCUTS))), group("Anywhere", table.unpack(keys_of(GLOBAL))))
end

local account_state = ui.shared("kx.account", nil :: any)

local function account(): Node
	local function a(): any
		return account_state:get() or {}
	end
	return page(
		"Account",
		group(
			"Keplorayx account",
			ui.switch(function(): string
				local s = a()
				return if s.signed_in then "in" elseif s.signing_in then "waiting" else "out"
			end, {
				out = setting(
					"Sign in with Woozlit",
					"Optional. Your account, for your phone and your other machines. Signing in sends no thread anywhere: Keplorayx stays local until you choose otherwise.",
					kx.Button {
						tone = "secondary",
						size = "sm",
						on_click = function()
							ui.invoke("kx.account.sign_in")
						end,
						"Sign in",
					}
				),
				waiting = setting(
					"Finish in your browser",
					"Sign in on the page that opened, then come back.",
					kx.Button {
						tone = "ghost",
						size = "sm",
						on_click = function()
							ui.invoke("kx.account.cancel")
						end,
						"Cancel",
					}
				),
				["in"] = ui.row {
					w_full = true,
					gap = "$space.3",
					items = "center",
					py = "$space.3",
					ui.row {
						w = 32,
						h = 32,
						rounded = "$radius.full",
						bg = "$accent",
						items = "center",
						justify = "center",
						ui.text {
							function()
								local e = a().email or ""
								return string.upper(string.sub(if e ~= "" then e else "K", 1, 1))
							end,
							literal = true,
							weight = "$font.weight.semibold",
							color = "$accent.text",
						},
					},
					ui.column {
						grow = 1,
						min_w = 0,
						ui.text {
							function()
								return a().email or "Signed in"
							end,
							literal = true,
							truncate = true,
						},
						kx.Hint {
							function()
								return if a().connected then "Connected" else "Signed in · connecting"
							end,
							literal = true,
						},
					},
					kx.Button {
						tone = "ghost",
						size = "sm",
						on_click = function()
							ui.invoke("kx.account.sign_out")
						end,
						"Sign out",
					},
				},
			})
		),
		ui.show(function()
			local e = a().error
			return e ~= nil and e ~= ""
		end, kx.Caption {
			function()
				return a().error or ""
			end,
			literal = true,
			wrap = true,
			color = "$danger",
		}),
		kx.Caption {
			"Woozlit is the account Keplorayx has always used, on the Cosmos edge. The agents' own sign-ins stay theirs, under Agents.",
			literal = true,
			wrap = true,
		}
	)
end

local data_dir = ui.shared("kx.data_dir", "")

local function data(): Node
	return page(
		"Privacy and data",
		group(
			"On this device",
			ui.row {
				w_full = true,
				gap = "$space.3",
				py = "$space.4",
				items = "start",
				ui.column { color = "$text.primary", pt = "$kx.space.hair", kx.Icon { name = "local-only", size = 20 } },
				ui.column {
					grow = 1,
					min_w = 0,
					gap = "$space.1",
					ui.text { "Local only", literal = true, weight = "$font.weight.medium" },
					kx.Caption {
						"Your projects, threads, settings and teammates are stored on this computer. Keplorayx sends none of it anywhere.",
						literal = true,
						wrap = true,
					},
				},
			},
			setting(
				"Where it lives",
				nil,
				ui.row {
					gap = "$space.2",
					items = "center",
					kx.Mono {
						function()
							return data_dir:get()
						end,
						literal = true,
						truncate = true,
						max_w = 260,
						text_size = "$font.size.sm",
						color = "$text.secondary",
					},
					kx.Button {
						tone = "secondary",
						size = "sm",
						on_click = function()
							pcall(ui.invoke, "open_url", data_dir:peek())
						end,
						"Open",
					},
				}
			)
		),
		group(
			"What does leave",
			setting("The agents you run", "Each sends your prompts and the files it reads to its own model provider.", nil),
			setting("Services you connect", "MCP servers, Supermemory and cloud machines see what you send them.", nil),
			setting("Update checks", "Asks npm and GitHub whether your agents have new versions.", nil)
		)
	)
end

local function about(): Node
	return page(
		"About",
		ui.row {
			gap = "$space.3",
			items = "center",
			ui.column { color = "$text.primary", kx.Icon { name = "orbit", size = 32 } },
			ui.column {
				gap = "$kx.space.hair",
				ui.text { "Keplorayx", literal = true, font = "$kx.display", text_size = "$font.size.xxl" },
				kx.Caption { "on zgx · development build", literal = true },
			},
		},
		kx.Body {
			"Every agent is a body in orbit around your work: calm while it works, drawn closer when it needs you.",
			literal = true,
			color = "$text.secondary",
		},
		kx.Caption {
			"Built on zgx and Dove's engine. Parts of the interface are ported from Zeron (MIT); the type is Geist and Instrument Serif (OFL). See THIRD_PARTY_NOTICES.md.",
			literal = true,
			wrap = true,
		}
	)
end

local PAGES: { [string]: () -> Node } = {
	general = general,
	appearance = appearance,
	agents = agents,
	notifications = notifications,
	shortcuts = shortcuts,
	account = account,
	data = data,
	about = about,
}

function M.view(): Node
	load_stored()
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
			ui.text { s.title, literal = true, grow = 1, text_size = "$font.size.md" },
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
		ui.row { h = "$kx.titlebar", w_full = true, window_drag = true },
		ui.row {
			grow = 1,
			min_h = 0,
			w_full = true,
			ui.column { w = 220, shrink = 0, px = "$space.3", gap = "$kx.space.hair", table.unpack(items) },
			ui.scroll {
				grow = 1,
				min_w = 0,
				h_full = true,
				ui.column {
					w_full = true,
					max_w = 680,
					px = "$space.6",
					pb = "$space.8",
					ui.switch(section, pages),
				},
			},
		},
	}
end

return M
