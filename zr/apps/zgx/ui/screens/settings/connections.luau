-- Settings → Connections: apps the person connected on their own Composio
-- account (Gmail, Slack, Notion, …), and who may use each: everywhere,
-- Zeron's chats only, every buddy, or some buddies; and whether an action
-- that changes something waits for their OK. The engine reads the same
-- policy (src/kx/connections.rs, crates/composio) to give a chat Composio
-- or not, and the bridge holds each chat to it.

local ds = require("@ds")
local Icon = require("@ds/icon")
local buddies = require("@app/kx/buddies/store")

local M = {}

M.ID = "connections"
M.TITLE = "Connections"

local policy = ui.shared("kx.connections", {} :: { [string]: any })

-- Apps most people connect first; any other is added by its Composio name.
local COMMON = {
	{ slug = "gmail", label = "Gmail" },
	{ slug = "googlecalendar", label = "Google Calendar" },
	{ slug = "googledrive", label = "Google Drive" },
	{ slug = "slack", label = "Slack" },
	{ slug = "notion", label = "Notion" },
	{ slug = "github", label = "GitHub" },
	{ slug = "linear", label = "Linear" },
	{ slug = "outlook", label = "Outlook" },
}

local function label_of(slug: string): string
	for _, a in COMMON do
		if a.slug == slug then
			return a.label
		end
	end
	return (string.gsub(slug, "^%l", string.upper))
end

local SCOPES = {
	{ value = "everywhere", label = "Everywhere" },
	{ value = "zeron", label = "Zeron chats" },
	{ value = "buddies", label = "All bots" },
	{ value = "some", label = "Some bots" },
}
local RULES = {
	{ value = "ask_changes", label = "Ask before changes" },
	{ value = "allow", label = "Allow without asking" },
	{ value = "always_ask", label = "Always ask" },
}

-- Whether two lists of buddy ids say the same.
local function same(a: { string }?, b: { string }?): boolean
	return table.concat(a or {}, ",") == table.concat(b or {}, ",")
end

local function copy(v: any): any
	if type(v) ~= "table" then
		return v
	end
	local out = {}
	for k, x in v do
		out[k] = copy(x)
	end
	return out
end

-- The policy, changed by `change` and saved.
local function save(change: (any) -> ())
	local p = copy(policy:peek() or {})
	p.key = nil
	p.apps = p.apps or {}
	p.buddies = p.buddies or {}
	change(p)
	ui.call("kx.connections.set", p):after(function() end, function(why: any)
		ds.toast.show { title = "It couldn't be saved", body = tostring(why), tone = "danger" }
	end)
end

-- Bots to choose from when an app is kept to some of them.
local function picker(chosen: () -> { string }, set: ({ string }) -> ()): Node
	return ui.each(buddies.list, function(b: any): Node
		local function on(): boolean
			return table.find(chosen(), b.id) ~= nil
		end
		return ui.button {
			label = b.name,
			h = 28,
			px = "$space.3",
			items = "center",
			rounded = "$radius.full",
			bg = function(): string
				return if on() then "$zeron.selected.flat.bg" else "$hover"
			end,
			on_click = function()
				local next = table.clone(chosen())
				local at = table.find(next, b.id)
				if at then
					table.remove(next, at)
				else
					table.insert(next, b.id)
				end
				set(next)
			end,
			ui.text { b.name, literal = true, text_size = "$font.size.sm", color = function(): string
				return if on() then "$text.primary" else "$text.secondary"
			end },
		}
	end, { direction = "row", flex_wrap = true, gap = "$space.1", px = "$space.4", pb = "$space.3", key = function(b: any): string
		return b.id .. b.name
	end })
end

local KeyRow = ui.component(function(): Node
	local key = ui.use_state("")
	local busy = ui.use_state(false)
	local function check()
		busy:set(true)
		ui.call("kx.composio.check"):after(function(n: any)
			busy:set(false)
			ds.toast.show { title = "Composio is connected", body = `{n} tools offered.`, tone = "success" }
		end, function(why: any)
			busy:set(false)
			ds.toast.show { title = "Composio didn't answer", body = tostring(why), tone = "danger" }
		end)
	end
	return ui.presence(function(): string
		return if (policy:get() or {}).key then "held" else "none"
	end, function(which: string): Node
		if which == "held" then
			return ds.Settings.Row {
				label = "Composio key",
				description = "Kept sealed on this PC. Chats and bots reach your connected apps through it.",
				control = ui.row {
					gap = "$space.2",
					ds.Button { tone = "secondary", size = "sm", disabled = function(): boolean
						return busy:get()
					end, on_click = check, ui.text { "Check", literal = true } },
					ds.Button { tone = "ghost", size = "sm", on_click = function()
						ui.invoke("kx.key.remove", "composio")
						ui.invoke("kx.connections.refresh")
					end, ui.text { "Remove", literal = true } },
				},
			}
		end
		return ui.column {
			w_full = true,
			px = "$space.4",
			py = "$space.3",
			gap = "$space.2",
			ui.text { "Composio key", literal = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.medium" },
			ui.row {
				w_full = true,
				gap = "$space.2",
				items = "center",
				ui.div { grow = 1, min_w = 0, ds.TextField { value = key, password = true, placeholder = "ck_…", w_full = true } },
				ds.Button {
					size = "sm",
					disabled = function(): boolean
						return key:get() == "" or busy:get()
					end,
					on_click = function()
						local value = key:peek()
						key:set("")
						ui.call("kx.key.set", { name = "composio", value = value }):after(function()
							ui.invoke("kx.connections.refresh")
							check()
						end, function(why: any)
							ds.toast.show { title = "The key wasn't kept", body = tostring(why), tone = "danger" }
						end)
					end,
					ui.text { "Save", literal = true },
				},
			},
			ui.row {
				gap = "$space.1",
				items = "center",
				ui.text { "From Composio's dashboard, under AI Clients.", literal = true, color = "$text.tertiary", text_size = "$font.size.xs" },
				ui.button {
					label = "Get a Composio key",
					on_click = function()
						pcall(ui.invoke, "open_url", "https://dashboard.composio.dev")
					end,
					ui.text { "Get a key", literal = true, color = "$accent", text_size = "$font.size.xs" },
				},
			},
		}
	end, { w_full = true })
end)

-- One connected app: who may use it, when it asks, and forgetting it.
local AppRow = ui.component(function(props: { slug: string, app: any }): Node
	local slug = props.slug
	local scope = ui.use_state(props.app.scope or "everywhere")
	local rule = ui.use_state(props.app.rule or "ask_changes")
	local chosen = ui.use_state(props.app.buddies or {})
	ui.effect(function()
		local s, r, c = scope:get(), rule:get(), chosen:get()
		if s ~= (props.app.scope or "everywhere") or r ~= (props.app.rule or "ask_changes") or not same(c, props.app.buddies) then
			save(function(p: any)
				p.apps[slug] = { scope = s, rule = r, buddies = c }
			end)
		end
	end)
	return ui.column {
		w_full = true,
		ui.row {
			w_full = true,
			px = "$space.4",
			py = "$space.3",
			gap = "$space.3",
			items = "center",
			ui.text { label_of(slug), literal = true, grow = 1, min_w = 0, truncate = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.medium" },
			ui.div { w = 150, ds.Select { value = scope, options = SCOPES } },
			ui.div { w = 190, ds.Select { value = rule, options = RULES } },
			ds.IconButton {
				icon = "close" :: any,
				label = `Forget {label_of(slug)} here`,
				size = "sm",
				tone = "quiet",
				on_click = function()
					ui.invoke("kx.composio.forget", slug)
				end,
			},
		},
		ui.show(function(): boolean
			return scope:get() == "some"
		end, picker(function(): { string }
			return chosen:get()
		end, function(next: { string })
			chosen:set(next)
		end)),
	}
end)

local function connect(slug: string, busy: State<string?>)
	busy:set(slug)
	ui.call("kx.composio.connect", slug):after(function(status: any)
		busy:set(nil)
		if status == "active" then
			ds.toast.show { title = `{label_of(slug)} is connected`, tone = "success" }
		else
			ds.toast.show { title = `Finish signing in to {label_of(slug)} in your browser`, body = "Then press Connect again to check.", tone = "info" }
		end
	end, function(why: any)
		busy:set(nil)
		ds.toast.show { title = `{label_of(slug)} couldn't be connected`, body = tostring(why), tone = "danger" }
	end)
end

local AddApps = ui.component(function(): Node
	local busy = ui.use_state(nil :: string?)
	local other = ui.use_state("")
	local chips: { [any]: any } = { w_full = true, flex_wrap = true, gap = "$space.1", px = "$space.4", py = "$space.3" }
	for _, a in COMMON do
		table.insert(chips, ui.show(function(): boolean
			local apps = (policy:get() or {}).apps or {}
			return apps[a.slug] == nil
		end, ds.Button {
			tone = "secondary",
			size = "sm",
			disabled = function(): boolean
				return busy:get() ~= nil
			end,
			on_click = function()
				connect(a.slug, busy)
			end,
			Icon { name = "plus", size = 12 },
			ui.text {
				function(): string
					return if busy:get() == a.slug then "Waiting…" else a.label
				end,
				literal = true,
			},
		}))
	end
	return ui.column {
		w_full = true,
		ui.row(chips),
		ui.row {
			w_full = true,
			px = "$space.4",
			pb = "$space.3",
			gap = "$space.2",
			items = "center",
			ui.div { grow = 1, min_w = 0, ds.TextField { value = other, placeholder = "Another app, by its Composio name (e.g. hubspot)", w_full = true } },
			ds.Button {
				size = "sm",
				disabled = function(): boolean
					return other:get() == "" or busy:get() ~= nil
				end,
				on_click = function()
					local slug = string.lower((string.gsub(other:peek(), "%s", "")))
					other:set("")
					connect(slug, busy)
				end,
				ui.text { "Connect", literal = true },
			},
		},
	}
end)

local Defaults = ui.component(function(): Node
	local p = policy:peek() or {}
	local scope = ui.use_state(p.scope or "everywhere")
	local rule = ui.use_state(p.rule or "ask_changes")
	local chosen = ui.use_state(p.buddies or {})
	ui.effect(function()
		local s, r, c = scope:get(), rule:get(), chosen:get()
		local now = policy:peek() or {}
		if s ~= (now.scope or "everywhere") or r ~= (now.rule or "ask_changes") or not same(c, now.buddies) then
			save(function(x: any)
				x.scope, x.rule, x.buddies = s, r, c
			end)
		end
	end)
	return ui.column {
		w_full = true,
		ds.Settings.Row { label = "Who may use them", description = "For every app without its own choice below.", control = ui.div { w = 170, ds.Select { value = scope, options = SCOPES } } },
		ui.show(function(): boolean
			return scope:get() == "some"
		end, picker(function(): { string }
			return chosen:get()
		end, function(next: { string })
			chosen:set(next)
		end)),
		ds.Settings.Row { label = "Before an action", description = "Reading always runs. Sending, posting, deleting or changing waits for your OK unless you say otherwise.", control = ui.div { w = 200, ds.Select { value = rule, options = RULES } } },
	}
end)

function M.view(): Node
	ui.invoke("kx.connections.refresh")
	return ds.Settings.Page {
		title = "Connections",
		ds.Settings.Section {
			title = "Composio",
			description = "Your own Composio account connects apps for every chat and bot. Free, with your own key.",
			ds.Settings.Group { KeyRow {} },
		},
		ui.show(function(): boolean
			return (policy:get() or {}).key == true
		end, ui.column {
			w_full = true,
			gap = "$space.6",
			ds.Settings.Section { title = "By default", ds.Settings.Group { Defaults {} } },
			ds.Settings.Section {
				title = "Connected apps",
				ui.presence(function(): string
					local keys = {}
					for slug, a in (policy:get() or {}).apps or {} do
						table.insert(keys, `{slug}:{a.scope}:{a.rule}:{table.concat(a.buddies or {}, ",")}`)
					end
					table.sort(keys)
					return table.concat(keys, "|")
				end, function(_: string): Node
					local rows: { [any]: any } = {}
					local apps = (policy:peek() or {}).apps or {}
					local slugs = {}
					for slug in apps do
						table.insert(slugs, slug)
					end
					table.sort(slugs)
					for _, slug in slugs do
						table.insert(rows, AppRow { slug = slug, app = apps[slug] })
					end
					if #rows == 0 then
						return ui.text { "None yet. Connect one below.", literal = true, px = "$space.4", color = "$text.tertiary", text_size = "$font.size.sm" }
					end
					return ds.Settings.Group(rows)
				end, { w_full = true }),
			},
			ds.Settings.Section { title = "Connect an app", ds.Settings.Group { AddApps {} } },
		}),
	}
end

return M
