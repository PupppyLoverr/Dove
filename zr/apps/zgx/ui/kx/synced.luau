--!strict
-- Your Woozlit account's sessions, as the old Keplorayx showed them: every
-- session any of your devices published to the account (src/kx_account.rs
-- keeps the helper's list in `kx.account.sessions`). A page of its own in
-- the main window (the sidebar's "From your account", a page under the
-- `kx` plugin name), its list grouped by folder; one chosen shows its
-- transcript, read from its chat room once and kept, and Continue here
-- starts a new chat with where it stood in the composer.
--
--   synced.row()      the sidebar's row ({ plugin, id, title, icon }), or nil
--                     while signed out
--   synced.page()     the page

local ds = require("@ds")
local Icon = require("@ds/icon")
local Markdown = require("@ds/markdown")
local ListRow = require("@ds/list_row").ListRow
local route = require("@app/screens/shell/route")
local composer = require("@app/screens/chat/shared")

local M = {}

M.PLUGIN = "kx"
M.PAGE = "account"

type Synced = {
	id: string,
	title: string?,
	cwd: string?,
	branch: string?,
	harness: string?,
	preview: string?,
	archived: boolean?,
	here: boolean?,
}
type Block = { kind: string, text: string?, name: string?, error: boolean? }
type Message = { role: string, blocks: { Block } }

local list = ui.shared("kx.account.sessions", {} :: { Synced })
local account = ui.shared("account", {} :: any)
-- The one open (its id), and its transcript as it comes.
local open = ui.state("kx_synced_open", nil :: string?)
local transcript = ui.state("kx_synced_transcript", nil :: { Message }?)
local loading = ui.state("kx_synced_loading", false)

local function signed_in(): boolean
	return (account:get() or {}).state == "signed_in"
end

-- Your account's sessions, for the sidebar's own list (signed out, none):
-- every one any device shared, none chosen by hand.
function M.all(): { Synced }
	if not signed_in() then
		return {}
	end
	return list:get() or {}
end

local function folder(path: string?): string
	if not path or path == "" then
		return "No folder"
	end
	local p = string.gsub(path, "\\", "/")
	p = string.gsub(p, "/+$", "")
	return string.match(p, "([^/]+)$") or p
end

local function title_of(s: Synced): string
	local t = s.title or ""
	return if t ~= "" then t else "Untitled session"
end

local function agent_name(h: string?): string
	local names = {
		claude = "Claude Code", claudeCode = "Claude Code", codex = "Codex", cursor = "Cursor",
		opencode = "OpenCode", gemini = "Gemini", devin = "Devin", hermes = "Hermes",
	}
	return if h and names[h] then names[h] else (h or "")
end

local function by_id(id: string?): Synced?
	for _, s in list:get() or {} do
		if s.id == id then
			return s
		end
	end
	return nil
end

local function choose(id: string)
	open:set(id)
	transcript:set(nil)
	loading:set(true)
	ui.call("kx.account.transcript", id):after(function(messages: any)
		if open:peek() ~= id then
			return
		end
		transcript:set(messages or {})
		loading:set(false)
	end, function()
		if open:peek() == id then
			transcript:set({})
			loading:set(false)
		end
	end)
end

-- All of them, on the account's page (the sidebar's list shows the first
-- few, and a row for the rest).
function M.show_all()
	open:set(nil)
	route.plugin_page(M.PLUGIN, M.PAGE)
end

-- One opened from the sidebar's list: its transcript, on the account's page.
function M.show(id: string)
	choose(id)
	route.plugin_page(M.PLUGIN, M.PAGE)
end

-- Where it stood, for the composer of a new chat: the last few turns, each
-- cut short, so the agent picks up the thread without a wall of text.
local function context(s: Synced, messages: { Message }): string
	local lines = { `I'm continuing "{title_of(s)}", a session from my Keplorayx account. Where it stood:`, "" }
	local first = math.max(1, #messages - 5)
	for i = first, #messages do
		local m = messages[i]
		local said = {}
		for _, b in m.blocks do
			if b.kind == "text" and b.text then
				table.insert(said, b.text)
			end
		end
		local text = table.concat(said, "\n")
		if text ~= "" then
			if #text > 600 then
				text = string.sub(text, 1, 600) .. "…"
			end
			table.insert(lines, `{if m.role == "user" then "Me" else "Agent"}: {text}`)
			table.insert(lines, "")
		end
	end
	table.insert(lines, "Please pick up from here: ")
	return table.concat(lines, "\n")
end

local function continue_here(s: Synced)
	local text = context(s, transcript:peek() or {})
	-- (In the project at its folder, if this computer has one there.)
	local project: string? = nil
	local all = (ui.shared("sessions", {} :: any):peek() or {}).projects or {}
	local want = string.lower(string.gsub(s.cwd or "", "\\", "/"))
	for _, p in all do
		if want ~= "" and p.path and string.lower(string.gsub(p.path, "\\", "/")) == want then
			project = p.id
		end
	end
	route.new_chat(project or "none")
	ui.after(80, function()
		if composer.showing() == "" then
			composer.text:set(text)
		end
	end)
end

-- One session's row: its title, the folder and agent it ran with, and the
-- last thing said.
local function row(s: Synced): Node
	local where = { folder(s.cwd) }
	if s.branch and s.branch ~= "" then
		table.insert(where, s.branch)
	end
	local agent = agent_name(s.harness)
	if agent ~= "" then
		table.insert(where, agent)
	end
	return ListRow {
		label = title_of(s),
		on_click = function()
			choose(s.id)
		end,
		ui.row {
			w_full = true,
			px = "$space.3",
			py = "$space.2",
			gap = "$space.3",
			items = "center",
			ui.column {
				grow = 1,
				min_w = 0,
				gap = 2,
				ui.text { title_of(s), literal = true, truncate = true, color = "$text.primary", text_size = "$font.size.md", weight = 500 },
				ui.text {
					if s.preview and s.preview ~= "" then s.preview else table.concat(where, " · "),
					literal = true,
					truncate = true,
					color = "$text.tertiary",
					text_size = "$font.size.sm",
				},
			},
			-- (At its end, the agent alone: where it ran is the line under the
			-- title when there's no last word, and the group's name besides.)
			ui.text { agent, literal = true, truncate = true, max_w = 160, color = "$text.tertiary", text_size = "$font.size.sm" },
			Icon { name = "alt-arrow-right" :: any, size = 14, color = "$text.tertiary" },
		},
	}
end

local function list_page(): Node
	local groups = ui.computed(function(): { { name: string, rows: { Synced } } }
		local by: { [string]: { Synced } } = {}
		local order: { string } = {}
		for _, s in list:get() or {} do
			if not s.archived then
				local k = folder(s.cwd)
				if not by[k] then
					by[k] = {}
					table.insert(order, k)
				end
				table.insert(by[k], s)
			end
		end
		table.sort(order, function(a, b)
			return string.lower(a) < string.lower(b)
		end)
		local out = {}
		for _, k in order do
			table.insert(out, { name = k, rows = by[k] })
		end
		return out
	end)
	return ui.scroll {
		w_full = true,
		h_full = true,
		ui.column {
			w_full = true,
			max_w = 760,
			mx = "auto",
			px = "$space.6",
			py = "$space.8",
			gap = "$space.6",
			ds.Settings.Page {
				title = "From your account",
				description = function(): string
					local n = #(list:get() or {})
					if n == 0 then
						return "Sessions your devices share to your Keplorayx account show up here. None yet."
					end
					return `{n} session{if n == 1 then "" else "s"} from your devices, kept on your Keplorayx account. Open one to read it or carry on here.`
				end,
				trail = ds.Button {
					tone = "ghost",
					size = "sm",
					on_click = function()
						pcall(ui.invoke, "kx.account.refresh")
					end,
					ui.row { gap = "$space.1", items = "center", Icon { name = "refresh" :: any, size = 14 }, ui.text { "Refresh", literal = true } },
				},
				ui.each(groups, function(g: any): Node
					return ds.Settings.Section {
						title = g.name,
						ui.column {
							w_full = true,
							gap = 0,
							table.unpack((function()
								local rows = {}
								for _, s in g.rows do
									table.insert(rows, row(s))
								end
								return rows
							end)()),
						},
					}
				end, {
					key = function(g: any): string
						return g.name .. ":" .. tostring(#g.rows)
					end,
				}),
			},
		},
	}
end

local function bubble(m: Message): Node
	local parts = {}
	for _, b in m.blocks do
		if b.kind == "text" then
			if m.role == "user" then
				table.insert(parts, ui.text { b.text or "", literal = true, wrap = true, color = "$text.primary", text_size = "$font.size.md", line_height = 20 })
			else
				table.insert(parts, Markdown { text = b.text or "" })
			end
		elseif b.kind == "tool" then
			table.insert(parts, ui.row {
				gap = "$space.2",
				items = "center",
				min_w = 0,
				Icon { name = (if b.error then "close-circle" else "check-circle") :: any, size = 13, color = if b.error then "$danger" else "$text.tertiary" },
				ui.text { b.name or "Used", literal = true, color = "$text.secondary", text_size = "$font.size.sm", weight = 500 },
				ui.text { b.text or "", literal = true, truncate = true, min_w = 0, color = "$text.tertiary", text_size = "$font.size.sm", font = "mono" },
			})
		elseif b.kind == "error" then
			table.insert(parts, ui.text { b.text or "", literal = true, wrap = true, color = "$danger", text_size = "$font.size.sm" })
		end
	end
	if m.role == "user" then
		return ui.row {
			w_full = true,
			justify = "end",
			ui.column {
				max_w = "80%",
				px = "$space.4",
				py = "$space.3",
				rounded = "$radius.xl",
				bg = "$zeron.secondary.bg",
				gap = "$space.2",
				table.unpack(parts),
			},
		}
	end
	return ui.column { w_full = true, gap = "$space.2", table.unpack(parts) }
end

local function transcript_page(id: string): Node
	local s = by_id(id) or { id = id }
	return ui.column {
		w_full = true,
		h_full = true,
		-- Its head: back, its title and where it ran, Continue here.
		-- (On the conversation's measure, so its edges line up under it.)
		ui.row {
			w_full = true,
			max_w = 760,
			mx = "auto",
			px = "$space.6",
			pt = "$space.4",
			pb = "$space.3",
			gap = "$space.3",
			items = "center",
			ds.IconButton {
				icon = "alt-arrow-left" :: any,
				label = "All sessions",
				size = "sm",
				tone = "quiet",
				on_click = function()
					open:set(nil)
				end,
			},
			ui.column {
				grow = 1,
				min_w = 0,
				gap = 1,
				ui.text { title_of(s), literal = true, truncate = true, color = "$text.primary", text_size = "$font.size.lg", weight = 600 },
				ui.text {
					folder(s.cwd) .. (if s.harness then " · " .. agent_name(s.harness) else ""),
					literal = true,
					truncate = true,
					color = "$text.tertiary",
					text_size = "$font.size.sm",
				},
			},
			ds.Button {
				tone = "primary",
				size = "sm",
				disabled = function(): boolean
					return loading:get()
				end,
				on_click = function()
					continue_here(s)
				end,
				ui.text { "Continue here", literal = true },
			},
		},
		ui.scroll {
			w_full = true,
			grow = 1,
			min_h = 0,
			ui.column {
				w_full = true,
				max_w = 760,
				mx = "auto",
				px = "$space.6",
				py = "$space.4",
				gap = "$space.5",
				ui.presence(function(): string
					local t = transcript:get()
					return if loading:get() then "loading" elseif t and #t > 0 then "messages" else "empty"
				end, function(state: string): Node
					if state == "loading" then
						return ui.row {
							w_full = true,
							py = "$space.8",
							justify = "center",
							gap = "$space.2",
							items = "center",
							ui.text { "Reading it from your account…", literal = true, color = "$text.tertiary", text_size = "$font.size.sm" },
						}
					elseif state == "empty" then
						return ds.EmptyState {
							icon = "chat-round-line",
							size = "sm",
							title = "Nothing to read here yet",
							body = "Its device hasn't shared the conversation, or it's still on its way. You can still carry on from its title.",
						}
					end
					local nodes = {}
					for _, m in transcript:peek() or {} do
						table.insert(nodes, bubble(m))
					end
					return ui.column { w_full = true, gap = "$space.5", table.unpack(nodes) }
				end, { w_full = true }),
			},
		},
	}
end

function M.page(): Node
	return ui.column {
		w_full = true,
		h_full = true,
		class = "kx-synced",
		ui.presence(function(): string
			return open:get() or ""
		end, function(id: string): Node
			if id == "" then
				return list_page()
			end
			return transcript_page(id)
		end, { w_full = true, h_full = true }),
	}
end

return M
