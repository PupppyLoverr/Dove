--!strict
-- Settings, laid out as Zeron's are (screens/settings; MIT, see
-- THIRD_PARTY_NOTICES.md): a quiet list of pages, and each page's groups as
-- lifted cards of rows, a name and a line of why on the left, the control
-- on the right. Every change applies as it's made. The account is
-- Keplorayx's own (Woozlit), optional and never on by itself; agents'
-- sign-ins are theirs and live under Agents.

local kx = require("@kx")
local ds = require("@ds")
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

-- Settings' pages under Zeron's headings (its screens/shell/settings: a
-- heading where the group changes, down the list).
local SECTIONS = {
	{ id = "general", title = "General", icon = "settings-minimalistic", group = "Personal" },
	{ id = "appearance", title = "Appearance", icon = "palette-search", group = "Personal" },
	{ id = "notifications", title = "Notifications", icon = "bell", group = "Personal" },
	{ id = "shortcuts", title = "Shortcuts", icon = "keyboard", group = "Personal" },
	{ id = "account", title = "Account", icon = "user", group = "Personal" },
	{ id = "agents", title = "Providers", icon = "bot", group = "Agents" },
	{ id = "plugins", title = "Plugins", icon = "widget-add", group = "Customize" },
	{ id = "widgets", title = "Widgets", icon = "widget", group = "Customize" },
	{ id = "data", title = "Privacy and data", icon = "local-only", group = "Keplorayx" },
	{ id = "about", title = "About & updates", icon = "info-circle", group = "Keplorayx" },
}

-- Zeron's settings pieces (ds/settings): a page at a reading width, its
-- sections under small titles, each a card of rows with hairlines between.
local Settings = require("@ds/settings")

local function setting(title: string, hint: string?, control: Node?): Node
	return Settings.Row { label = title, description = hint, control = control }
end

local function group(title: string?, ...: Node): Node
	local section: { [any]: any } = { title = title }
	local card: { [any]: any } = {}
	for i, r in { ... } do
		card[i] = r
	end
	section[1] = Settings.Group(card)
	return Settings.Section(section)
end

local DESCRIPTIONS: { [string]: string } = {
	General = "You, and Keplorayx beside your other apps.",
	Appearance = "How Keplorayx looks: its light, its glass, your star, the sky.",
	Providers = "The agents on this computer, and what a new thread may do.",
	Notifications = "When Keplorayx says something while you're away.",
	Shortcuts = "Every key, in the window and from anywhere.",
	Account = "Your Keplorayx account, optional.",
	Plugins = "What you add to Keplorayx. A plugin runs only once you allow it, with exactly what it asked for.",
	Widgets = "Small windows on your desktop. You and your agents can add, change and move them.",
	["Privacy and data"] = "What stays on this computer, and what leaves it.",
	["About & updates"] = "Keplorayx, and what it's built on.",
}

local function page(title: string, ...: Node): Node
	local p: { [any]: any } = { title = title, description = DESCRIPTIONS[title] }
	for i, child in { ... } do
		p[i] = child
	end
	return Settings.Page(p)
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
	local tray = stored_flag("kx.keep_in_tray", true)
	local notch = stored_flag("kx.notch", true)
	local ok, shared = pcall(ui.shared, "kx.autostart", false)
	local login = ui.state("settings.autostart", if ok then (shared :: any):peek() == true else false)
	return page(
		"General",
		group(
			nil,
			setting(
				"Send messages with",
				"Enter, or Ctrl+Enter (then Enter starts a new line).",
				kx.Segmented {
					value = ui.persist("kx.send_with", "enter"),
					label = "Send messages with",
					width = 96,
					options = { { label = "Enter", value = "enter" }, { label = "Ctrl+Enter", value = "mod_enter" } },
				}
			),
			setting(
				"Compact mode",
				"A thread a line in the sidebar; off, two (its project and branch under it).",
				kx.Switch { value = require("@screens/nav").sidebar_compact, label = "Compact mode" }
			),
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
				"Alt+Space opens the launcher; Ctrl+Alt+Space brings Keplorayx forward to ask; Ctrl+Alt+A and Ctrl+Alt+D allow or deny what an agent waits on. Applies at the next launch.",
				kx.Switch {
					value = keys,
					label = "Keys from any app",
					on_change = function(on: boolean)
						store.set_setting("kx.global_keys", on)
					end,
				}
			),
			setting(
				"Keep running in the tray",
				"Closing the window leaves your agents working and the notch where it is. The icon by the clock brings it back; Quit is in its menu.",
				kx.Switch {
					value = tray,
					label = "Keep running in the tray",
					on_change = function(on: boolean)
						store.set_setting("kx.keep_in_tray", on)
					end,
				}
			),
			setting(
				"The notch",
				"A capsule at the top of the screen: your agents at a glance, and quiet when none are working.",
				kx.Switch {
					value = notch,
					label = "Show the notch",
					on_change = function(on: boolean)
						store.set_setting("kx.notch", on)
						pcall(ui.invoke, "kx.notch_visible", on)
					end,
				}
			),
			setting(
				"Start with Windows",
				"Keplorayx starts in the tray when you sign in to Windows, the window hidden until you open it.",
				kx.Switch {
					value = login,
					label = "Start with Windows",
					on_change = function(on: boolean)
						pcall(ui.invoke, "kx.autostart", on)
					end,
				}
			)
		)
	)
end

local function appearance(): Node
	return page(
		"Appearance",
		group(
			"Reading",
			setting(
				"Conversation width",
				"How wide a thread's replies run.",
				kx.Segmented {
					value = ui.persist("kx.conversation_width", "default"),
					label = "Conversation width",
					width = 84,
					options = {
						{ label = "Narrow", value = "narrow" },
						{ label = "Default", value = "default" },
						{ label = "Wide", value = "wide" },
					},
				}
			)
		),
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
			"New thread",
			setting(
				"Composer",
				"In the middle under the greeting, or at the foot as in a thread; slim is one line until you write more.",
				kx.Segmented {
					value = require("@screens/home").place,
					label = "Composer",
					width = 96,
					options = require("@screens/home").PLACES,
				}
			),
			setting(
				"Sky",
				"What a new thread sits over.",
				kx.Segmented {
					value = require("@screens/home").sky,
					label = "Sky",
					width = 72,
					options = require("@screens/home").SKIES,
				}
			),
			setting(
				"Effect",
				"How the sky is drawn.",
				kx.Segmented {
					value = require("@screens/home").sky_effect,
					label = "Effect",
					width = 72,
					options = {
						{ label = "Dither", value = "dither" },
						{ label = "ASCII", value = "ascii" },
						{ label = "Halftone", value = "halftone" },
						{ label = "Lines", value = "scanlines" },
						{ label = "None", value = "none" },
					},
				}
			)
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
						{ label = "Ask first", value = "supervised" },
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
			"Desktop",
			setting(
				"Input required",
				"When an agent asks you something or waits for your approval.",
				kx.Switch { value = ui.persist("kx.notify.needs", true), label = "Input required" }
			),
			setting(
				"Task completed",
				"When a thread finishes its turn.",
				kx.Switch { value = ui.persist("kx.notify.done", true), label = "Task completed" }
			),
			setting(
				"Only when in the background",
				"Off, they come while you're looking at Keplorayx too.",
				kx.Switch { value = ui.persist("kx.notify.background", true), label = "Only when in the background" }
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
	{ "Open a thread in the sidebar", "Ctrl 1 to 9" },
	{ "Allow what an agent asks", "Y" },
	{ "Deny it", "N" },
}

local GLOBAL = {
	{ "The launcher, from any app", "Alt Space" },
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

-- ── Plugins (src/plugins.rs) ───────────────────────────────────────────
-- Zeron's Plugins page (its screens/plugins/page.luau, after its gallery
-- example, MIT): the page's search at its title's end, Installed and
-- Library as tabs, each plugin a row with its tile, version, and what it
-- can do at its end: Install, a switch once allowed, Allow while it waits
-- (a dialog saying what it may do, as Zeron's consent window does), Remove
-- behind a confirm. Nothing runs until you allow it.
type PluginRow = {
	id: string,
	name: string,
	version: string?,
	description: string?,
	permissions: { { id: string, says: string } },
	installed: boolean,
	on: boolean,
	asks_more: boolean?,
	error: string?,
	mention: string?,
	tools: boolean?,
	requires: any?,
}
local plugin_list: State<{ PluginRow }> = if store.fixture ~= ""
	then ui.state("fx.plugins", {
		{
			id = "computer",
			name = "Computer use",
			version = "0.1.0",
			description = "Lets any agent see and work this computer's apps and browser, with @computer in the composer.",
			permissions = { { id = "tools", says = "Give agents tools (a program of its own)" } },
			installed = false,
			on = false,
			mention = "computer",
			tools = true,
		},
		{
			id = "focus-timer",
			name = "Focus timer",
			version = "1.0.0",
			description = "A timer under the sidebar's actions.",
			permissions = { { id = "ui.add", says = "Add to Keplorayx's window" } },
			installed = true,
			on = true,
		},
	} :: { PluginRow })
	else ui.shared("kx.plugins", {} :: { PluginRow })

local plugins_tab = ui.state("plugins.tab", "installed")
local plugins_query = ui.state("plugins.query", "")
local allowing = ui.state("plugins.allowing", nil :: PluginRow?)
local allowing_open = ui.state("plugins.allowing_open", false)
local removing = ui.state("plugins.removing", nil :: PluginRow?)
local removing_open = ui.state("plugins.removing_open", false)

local PLUGIN_HUES = { "violet", "indigo", "blue", "sky", "teal", "emerald", "rose", "amber" }
local function plugin_hue(id: string): string
	if id == "computer" then
		return "violet"
	end
	local h = 0
	for i = 1, #id do
		h = (h * 31 + string.byte(id, i)) % 2147483647
	end
	return PLUGIN_HUES[h % #PLUGIN_HUES + 1]
end

local function plugin_found(installed: boolean): State<{ PluginRow }>
	return ui.computed(function(): { PluginRow }
		local q = string.lower(plugins_query:get())
		local out = {}
		for _, p in plugin_list:get() do
			local text = string.lower(p.name .. " " .. (p.description or ""))
			if p.installed == installed and (q == "" or string.find(text, q, 1, true) ~= nil) then
				table.insert(out, p)
			end
		end
		return out
	end)
end
local installed_found = plugin_found(true)
local library_found = plugin_found(false)

local function permission_words(p: PluginRow): string
	local says: { string } = {}
	for _, perm in p.permissions or {} do
		table.insert(says, perm.says)
	end
	return table.concat(says, " · ")
end

local function plugin_row(p: PluginRow): Node
	local control: Node
	if not p.installed then
		control = ds.Button {
			"Install",
			tone = "secondary",
			size = "sm",
			on_click = function()
				pcall(ui.invoke, "kx.plugins.install", p.id)
				plugins_tab:set("installed")
				kx.toast { title = `{p.name} installed`, body = "Allow it to turn it on." }
			end,
		}
	elseif p.error then
		control = ds.IconButton {
			icon = "trash-bin-minimalistic",
			label = `Remove {p.name}`,
			size = "sm",
			tone = "quiet",
			on_click = function()
				removing:set(p)
				removing_open:set(true)
			end,
		}
	else
		local on = ui.state(`plugins.on.{p.id}`, p.on)
		control = ui.row {
			gap = "$space.2",
			items = "center",
			if p.on
				then ds.Switch {
					value = on,
					label = p.name,
					on_change = function(v: boolean)
						if not v then
							pcall(ui.invoke, "kx.plugins.allow", { id = p.id, on = false })
						end
					end,
				}
				else ds.Button {
					if p.asks_more then "Review" else "Allow",
					tone = "primary",
					size = "sm",
					on_click = function()
						allowing:set(p)
						allowing_open:set(true)
					end,
				},
			ds.IconButton {
				icon = "trash-bin-minimalistic",
				label = `Remove {p.name}`,
				size = "sm",
				tone = "quiet",
				on_click = function()
					removing:set(p)
					removing_open:set(true)
				end,
			},
		}
	end
	local about = p.error or p.description or ""
	if p.mention and p.installed and not p.error then
		about ..= ` Type @{p.mention} in the composer.`
	end
	local needs = if type(p.requires) == "table" then p.requires.says else nil
	if needs and not p.on then
		about ..= `\n{needs}`
	end
	return ds.Settings.Row {
		label = p.name,
		description = about,
		lead = ds.Tile { label = p.name, hue = plugin_hue(p.id), size = 32 },
		badge = ds.Badge { p.version or "", tabular = true },
		control = control,
	}
end

local function plugin_list_of(found: State<{ PluginRow }>): Node
	return ds.Settings.Each(found, plugin_row, {
		key = function(p: PluginRow): string
			return `{p.id}:{p.on}:{p.installed}:{p.asks_more or false}:{p.error or ""}`
		end,
	})
end

local function plugins_body(key: string): Node
	if key == "none" then
		return ds.EmptyState {
			icon = "widget",
			title = "No plugins yet",
			body = "Plugins give your agents tools and add to Keplorayx. Computer use is in the library; agents can make you more.",
			action = {
				label = "Open the library",
				on_click = function()
					plugins_tab:set("library")
				end,
			},
		}
	elseif string.match(key, ":nothing$") then
		return ds.EmptyState {
			icon = "search",
			title = function(): string
				return if plugins_query:get() == "" then "Everything's installed" else `Nothing matches “{plugins_query:get()}”`
			end,
			body = "Try another word, or look in the other tab.",
			size = "sm",
			action = {
				label = "Clear search",
				on_click = function()
					plugins_query:set("")
				end,
			},
		}
	end
	return plugin_list_of(if string.match(key, "^list:library") then library_found else installed_found)
end

local function plugins_page(): Node
	return ui.column {
		w_full = true,
		Settings.Page {
			title = "Plugins",
			description = DESCRIPTIONS.Plugins,
			trail = ds.SearchField { value = plugins_query, placeholder = "Search plugins", size = "sm", w = 200 },
			ui.column {
				w_full = true,
				gap = "$space.4",
				ds.Tabs {
					value = plugins_tab,
					label = "Plugins",
					options = {
						{ value = "installed", label = "Installed", count = function(): number
							return #installed_found:get()
						end },
						{ value = "library", label = "Library", count = function(): number
							return #library_found:get()
						end },
					},
				},
				ui.presence(function(): string
					local t = plugins_tab:get()
					if t == "installed" and #plugin_list:get() == 0 then
						return "none"
					end
					local found = if t == "library" then library_found:get() else installed_found:get()
					return if #found > 0 then `list:{t}` else `list:{t}:nothing`
				end, plugins_body, { w_full = true, enter = { opacity = { 0, 1 }, duration = 120 }, exit = { opacity = 0, duration = 60 }, mode = "wait" }),
				Settings.Section {
					title = "Make your own",
					Settings.Group {
						setting(
							"Ask an agent",
							"Agents make and edit plugins with Keplorayx's tools; what they make waits here for you to allow it. Or put a folder with kx-plugin.json and init.luau in the plugins folder.",
							ds.Button {
								"Open the folder",
								tone = "secondary",
								size = "sm",
								on_click = function()
									pcall(ui.invoke, "kx.plugins.folder")
								end,
							}
						),
					},
				},
			},
		},
		ds.Dialog {
			open = allowing_open,
			title = function(): string
				local p = allowing:get()
				return if p then `Allow {p.name}?` else ""
			end,
			body = function(): string
				local p = allowing:get()
				if not p then
					return ""
				end
				local words = permission_words(p)
				return if words == "" then "It asks for nothing beyond its own sandbox." else `It will be able to: {words}.`
			end,
			confirm = {
				label = "Allow",
				on_click = function()
					local p = allowing:peek()
					if p then
						pcall(ui.invoke, "kx.plugins.allow", { id = p.id, on = true })
						kx.toast { title = `{p.name} is on` }
					end
				end,
			},
		},
		ds.Dialog {
			open = removing_open,
			tone = "danger",
			title = function(): string
				local p = removing:get()
				return if p then `Remove {p.name}?` else ""
			end,
			body = "Its folder goes, and agents lose any tools it gave them. You can install it again from the library.",
			confirm = {
				label = "Remove",
				on_click = function()
					local p = removing:peek()
					if p then
						pcall(ui.invoke, "kx.plugins.remove", p.id)
					end
				end,
			},
		},
	}
end

-- ── Widgets (src/widgets.rs) ────────────────────────────────────────────
type WidgetRow = { id: string, kind: string, variant: string, title: string?, settings: any? }
local widget_list: State<{ WidgetRow }> = if store.fixture ~= ""
	then ui.state("fx.widgets", {
		{ id = "w1", kind = "clock", variant = "digital" },
		{ id = "w2", kind = "calendar", variant = "agenda" },
	} :: { WidgetRow })
	else ui.shared("kx.widgets", {} :: { WidgetRow })

local WIDGET_KINDS = {
	{ id = "clock", label = "Clock", variants = { "digital", "analog", "world", "small" } },
	{ id = "calendar", label = "Calendar", variants = { "agenda", "month", "small" } },
	{ id = "agents", label = "Agents at work", variants = { "medium", "small", "large" } },
	{ id = "threads", label = "Recent threads", variants = { "medium", "small", "large" } },
	{ id = "ask", label = "Quick ask", variants = { "medium" } },
	{ id = "usage", label = "Usage", variants = { "medium", "small", "large" } },
	{ id = "system", label = "CPU and memory", variants = { "bars", "small", "graph" } },
	{ id = "music", label = "Now playing", variants = { "medium", "small" } },
	{ id = "note", label = "Note", variants = { "medium", "small", "large" } },
}

local function kind_of(id: string): any?
	for _, k in WIDGET_KINDS do
		if k.id == id then
			return k
		end
	end
	return nil
end

local function note_field(w: WidgetRow): Node
	local text = ui.state(`widgets.note.{w.id}`, (if type(w.settings) == "table" and type(w.settings.text) == "string" then w.settings.text else "") :: string)
	return kx.Field {
		value = text,
		placeholder = "The note",
		label = "The note",
		w = 220,
		on_commit = function()
			local s = if type(w.settings) == "table" then table.clone(w.settings) else {}
			s.text = text:peek()
			pcall(ui.invoke, "kx.widget.set", { id = w.id, kind = w.kind, variant = w.variant, settings = s })
		end,
	}
end

local function widget_row(w: WidgetRow): Node
	local k = kind_of(w.kind)
	local options: { any } = {}
	for _, v in (if k then k.variants else {}) do
		table.insert(options, { label = string.upper(string.sub(v, 1, 1)) .. string.sub(v, 2), value = v })
	end
	local variant = ui.state(`widgets.variant.{w.id}`, w.variant)
	return setting(
		w.title or (if k then k.label elseif w.kind == "custom" then "Made by an agent" else w.kind),
		"On the desktop, under your windows. Drag it on the desktop to move it.",
		ui.row {
			gap = "$space.2",
			items = "center",
			-- A note's words: written here (the desktop never takes the keys).
			if w.kind == "note" then note_field(w) else nil,
			if #options > 1
				then kx.Segmented {
					value = variant,
					label = "Variant",
					options = options,
					on_change = function(v: string)
						pcall(ui.invoke, "kx.widget.set", { id = w.id, kind = w.kind, variant = v })
					end,
				}
				else nil,
			kx.Button {
				tone = "ghost",
				size = "sm",
				on_click = function()
					pcall(ui.invoke, "kx.widget.remove", w.id)
				end,
				"Remove",
			},
		}
	)
end

local function widgets_page(): Node
	local add_row: { [any]: any } = { gap = "$space.2", wrap = true, w_full = true, py = "$space.3", px = "$space.4" }
	for _, k in WIDGET_KINDS do
		table.insert(
			add_row,
			kx.Button {
				tone = "secondary",
				size = "sm",
				on_click = function()
					pcall(ui.invoke, "kx.widget.set", { kind = k.id, variant = k.variants[1] })
				end,
				k.label,
			}
		)
	end
	return page(
		"Widgets",
		group(
			"On your desktop",
			ui.show(
				function()
					return #widget_list:get() > 0
				end,
				ui.column {
					w_full = true,
					ui.each(widget_list, widget_row, {
						key = function(w: WidgetRow)
							return `{w.id}:{w.variant}`
						end,
					}),
				},
				setting("None yet", "Add one below, or ask an agent to put a clock and your calendar on the desktop.", nil)
			)
		),
		group("Add", ui.row(add_row))
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
	plugins = plugins_page,
	widgets = widgets_page,
	data = data,
	about = about,
}

-- Settings' pages, in the sidebar's place while Settings shows (Zeron's
-- way: one list down the side, never two): Back first, then each page.
function M.nav(): Node
	local nav = require("@screens/nav")
	local items: { any } = {}
	local last: string? = nil
	for _, s in SECTIONS do
		if s.group ~= last then
			table.insert(items, { heading = s.group })
			last = s.group
		end
		table.insert(items, { value = s.id, label = s.title, icon = s.icon })
	end
	return Settings.Nav {
		value = section,
		items = items,
		width = nav.sidebar_width,
		top = 40,
		lead = { { value = "::back", label = "Back to threads", icon = "arrow-left" } },
		on_choose = function(value: any)
			if value == "::back" then
				nav.go("home")
			else
				section:set(value)
			end
		end,
	}
end

function M.view(): Node
	load_stored()
	local pages: { [string]: Node } = {}
	for id, build in PAGES do
		pages[id] = ui.column { w_full = true, build() }
	end
	return ui.scroll {
		grow = 1,
		min_w = 0,
		h_full = true,
		ui.column {
			w_full = true,
			items = "center",
			pt = "$space.7",
			px = "$space.6",
			pb = "$space.8",
			ui.switch(section, pages),
		},
	}
end

return M
