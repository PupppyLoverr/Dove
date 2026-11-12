--!strict
-- Keplorayx's own pages in the main window, each a page under the `kx`
-- plugin name (the shell's page asks here first: screens/shell/page):
--
--   account     a session from your account, read (ui/kx/synced)
--   kanban      every session on a board, by how it stands
--   artifacts   what agents made in chats with no project
--
--   pages.page(id)   the page, or nil for none of these

local ds = require("@ds")
local Icon = require("@ds/icon")
local ListRow = require("@ds/list_row").ListRow
local status = require("@ds/status")
local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")

local M = {}

M.PLUGIN = "kx"

-- ── Kanban ──────────────────────────────────────────────────────────────
-- Four lanes, by a session's state: what waits on you, what's at work,
-- what's done and unseen, and the rest.
type Lane = { id: string, title: string, note: string, holds: { [string]: boolean } }
local LANES: { Lane } = {
	{ id = "needs", title = "Needs you", note = "Waiting on an answer or an approval", holds = { approval = true, input = true, failed = true, limited = true } },
	{ id = "working", title = "Working", note = "Agents at work now", holds = { working = true, connecting = true, plan = true, waiting = true } },
	{ id = "done", title = "Done", note = "Finished since you last looked", holds = { done = true } },
	{ id = "idle", title = "Idle", note = "Nothing going on", holds = {} },
}

local function lane_of(state: string?): string
	for _, l in LANES do
		if state and l.holds[state] then
			return l.id
		end
	end
	return "idle"
end

type Card = { id: string, title: string, where: string, updated: number }

local function cards(lane: string): { Card }
	local now = sessions.now:get()
	local names: { [string]: string } = {}
	for _, p in now.projects do
		names[p.id] = p.name
	end
	local out: { Card } = {}
	for _, c in now.chats do
		if lane_of(c.status) == lane then
			table.insert(out, {
				id = c.id,
				title = if c.title ~= "" then c.title else "Untitled session",
				where = (c.project and names[c.project]) or "No project",
				updated = c.updated,
			})
		end
	end
	table.sort(out, function(a: Card, b: Card): boolean
		return a.updated > b.updated
	end)
	return out
end

local function card(c: Card): Node
	return ui.button {
		label = c.title,
		w_full = true,
		shrink = 0,
		px = "$space.3",
		py = "$space.3",
		rounded = "$radius.lg",
		bg = "$zeron.secondary.bg",
		hover = { bg = "$zeron.secondary.hover" },
		border = 1,
		border_color = "$border",
		on_click = function()
			route.open_session(c.id)
		end,
		ui.column {
			w_full = true,
			min_w = 0,
			gap = "$space.1",
			items = "start",
			ui.text { c.title, literal = true, truncate = true, w_full = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.medium" },
			ui.text {
				`{c.where} · {status.ago(math.max(0, os.time() - c.updated))}`,
				literal = true,
				truncate = true,
				w_full = true,
				color = "$text.tertiary",
				text_size = "$font.size.sm",
			},
		},
	}
end

local function lane(l: Lane): Node
	local held = ui.computed(function(): { Card }
		return cards(l.id)
	end)
	return ui.column {
		-- (Four lanes of one width, whatever each holds.)
		w = "22%",
		grow = 1,
		min_w = 0,
		h_full = true,
		gap = "$space.3",
		ui.row {
			w_full = true,
			shrink = 0,
			px = "$space.1",
			gap = "$space.2",
			items = "center",
			ui.text { l.title, literal = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.semibold" },
			ui.text {
				function(): string
					return tostring(#held:get())
				end,
				literal = true,
				color = "$text.tertiary",
				text_size = "$font.size.sm",
				tabular = true,
			},
		},
		ui.scroll {
			w_full = true,
			grow = 1,
			min_h = 0,
			ui.column {
				w_full = true,
				gap = "$space.2",
				pb = "$space.6",
				ui.show(function(): boolean
					return #held:get() == 0
				end, ui.text { l.note, literal = true, wrap = true, px = "$space.1", color = "$text.tertiary", text_size = "$font.size.sm" }),
				ui.each(held, function(c: any): Node
					return card(c)
				end, {
					key = function(c: any): string
						return `{c.id}:{c.title}:{c.where}:{c.updated}`
					end,
				}),
			},
		},
	}
end

local function kanban(): Node
	local lanes: { [any]: any } = {
		w_full = true,
		grow = 1,
		min_h = 0,
		gap = "$space.4",
	}
	for _, l in LANES do
		table.insert(lanes, lane(l))
	end
	return ui.column {
		class = "kx-kanban",
		w_full = true,
		h_full = true,
		px = "$space.6",
		pt = "$space.8",
		gap = "$space.5",
		ui.column {
			w_full = true,
			shrink = 0,
			gap = "$space.1",
			ui.text { "Kanban", literal = true, color = "$text.primary", text_size = "$font.size.xl", weight = "$font.weight.semibold" },
			ui.text {
				"Every session by how it stands. Open one to carry on with it.",
				literal = true,
				color = "$text.secondary",
				text_size = "$font.size.md",
			},
		},
		ui.row(lanes),
	}
end

-- ── Artifacts ───────────────────────────────────────────────────────────
-- What agents made in chats with no project: each such chat works in a
-- folder of its own in Keplorayx's data (src/kx/artifacts.rs lists them,
-- newest first). Narrowed by kind; one pressed opens in what the system
-- opens it with.
type Artifact = { name: string, path: string, folder: string, rel: string, kind: string, size: number, modified: number }
local found = ui.state("kx_artifacts", {} :: { Artifact })
local found_once = ui.state("kx_artifacts_read", false)
local kind_shown = ui.state("kx_artifacts_kind", "all")

local function read_artifacts()
	ui.call("kx.artifacts.list"):after(function(list: any)
		found:set(if type(list) == "table" then list else {})
		found_once:set(true)
	end, function()
		found_once:set(true)
	end)
end
-- (Read as the window's scripts load, for the sidebar's count; again as
-- the page opens, and by its Refresh.)
read_artifacts()

-- How many there are (the sidebar's Artifacts row says it).
function M.artifact_count(): number
	return #found:get()
end

local KINDS: { { id: string, title: string, icon: string } } = {
	{ id = "all", title = "All", icon = "layers" },
	{ id = "document", title = "Documents", icon = "document" },
	{ id = "page", title = "Pages", icon = "layers" },
	{ id = "image", title = "Images", icon = "gallery" },
	{ id = "data", title = "Data", icon = "chart" },
	{ id = "code", title = "Code", icon = "document" },
	{ id = "other", title = "Other", icon = "document" },
}
local KIND_ICON: { [string]: string } = {}
for _, k in KINDS do
	KIND_ICON[k.id] = k.icon
end

local function size_text(bytes: number): string
	if bytes < 1024 then
		return `{bytes} B`
	elseif bytes < 1024 * 1024 then
		return `{math.floor(bytes / 1024 + 0.5)} KB`
	end
	return string.format("%.1f MB", bytes / (1024 * 1024))
end

local function kind_chip(k: { id: string, title: string, icon: string }): Node
	local function count(): number
		if k.id == "all" then
			return #found:get()
		end
		local n = 0
		for _, f in found:get() do
			if f.kind == k.id then
				n += 1
			end
		end
		return n
	end
	return ui.button {
		label = k.title,
		h = 28,
		px = "$space.3",
		shrink = 0,
		gap = "$space.2",
		items = "center",
		rounded = "$radius.full",
		border = 1,
		border_color = function(): string
			return if kind_shown:get() == k.id then "$zeron.selected.ring" else "$border"
		end,
		bg = function(): string
			return if kind_shown:get() == k.id then "$zeron.selected.bg" else "$zeron.secondary.bg"
		end,
		hover = { bg = "$zeron.secondary.hover" },
		on_click = function()
			kind_shown:set(k.id)
		end,
		ui.text { k.title, literal = true, color = "$text.primary", text_size = "$font.size.sm", weight = "$font.weight.medium" },
		ui.text {
			function(): string
				return tostring(count())
			end,
			literal = true,
			color = "$text.tertiary",
			text_size = "$font.size.sm",
			tabular = true,
		},
	}
end

local function artifact_row(f: Artifact): Node
	return ListRow {
		label = f.name,
		on_click = function()
			pcall(ui.invoke, "kx.artifacts.open", f.path)
		end,
		ui.row {
			w_full = true,
			px = "$space.3",
			py = "$space.2",
			gap = "$space.3",
			items = "center",
			ui.column { shrink = 0, color = "$text.secondary", Icon { name = (KIND_ICON[f.kind] or "document") :: any, size = 18 } },
			ui.column {
				grow = 1,
				min_w = 0,
				gap = 2,
				ui.text { f.name, literal = true, truncate = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.medium" },
				ui.text {
					`{f.folder}/{f.rel} · {size_text(f.size)}`,
					literal = true,
					truncate = true,
					color = "$text.tertiary",
					text_size = "$font.size.sm",
				},
			},
			ui.text {
				status.ago(math.max(0, os.time() - f.modified)),
				literal = true,
				shrink = 0,
				color = "$text.tertiary",
				text_size = "$font.size.sm",
				tabular = true,
			},
		},
	}
end

local function artifacts(): Node
	read_artifacts()
	local shown = ui.computed(function(): { Artifact }
		local kind = kind_shown:get()
		local out: { Artifact } = {}
		for _, f in found:get() do
			if kind == "all" or f.kind == kind then
				table.insert(out, f)
			end
		end
		return out
	end)
	local chips: { [any]: any } = { w_full = true, shrink = 0, gap = "$space.2", items = "center" }
	for _, k in KINDS do
		table.insert(chips, kind_chip(k))
	end
	return ui.scroll {
		class = "kx-artifacts",
		w_full = true,
		h_full = true,
		ui.column {
			w_full = true,
			max_w = 860,
			mx = "auto",
			px = "$space.6",
			py = "$space.8",
			gap = "$space.5",
			ui.row {
				w_full = true,
				gap = "$space.3",
				items = "start",
				ui.column {
					grow = 1,
					min_w = 0,
					gap = "$space.1",
					ui.text { "Artifacts", literal = true, color = "$text.primary", text_size = "$font.size.xl", weight = "$font.weight.semibold" },
					ui.text {
						"Files agents made in chats that have no project, newest first. Open one in what your computer opens it with.",
						literal = true,
						wrap = true,
						color = "$text.secondary",
						text_size = "$font.size.md",
					},
				},
				ds.Button {
					tone = "ghost",
					size = "sm",
					on_click = read_artifacts,
					ui.row { gap = "$space.1", items = "center", Icon { name = "refresh" :: any, size = 14 }, ui.text { "Refresh", literal = true } },
				},
			},
			ui.row(chips),
			ui.show(function(): boolean
				return found_once:get() and #shown:get() == 0
			end, ds.EmptyState {
				icon = "layers",
				size = "sm",
				title = "Nothing here yet",
				body = "When an agent writes a file in a chat with no project (a document, a picture, a page), it's listed here.",
			}),
			ui.column {
				w_full = true,
				gap = 0,
				ui.each(shown, function(f: any): Node
					return artifact_row(f)
				end, {
					key = function(f: any): string
						return `{f.path}:{f.modified}:{f.size}`
					end,
				}),
			},
		},
	}
end

-- ── The sidebar's tabs (ui/kx/nav), as the sidebar asks for them ─────────
--   pages.buddies_row()          Buddies
--   pages.more_rows(filter_on)   More, and what it holds while it's open
--   pages.open(id)          what a click on one does (Projects is the
--                           sidebar's own: its filter); false for none
-- The Buddies tab: lit while the Buddies space shows.
function M.buddies_row(): any
	return {
		plugin = M.PLUGIN,
		id = "buddies",
		title = "Buddies",
		icon = "users-group-rounded",
		selected = require("@app/kx/buddies/sidebar").active(),
	}
end

function M.more_rows(filter_on: boolean): { any }
	local rows = require("@app/kx/nav").more(filter_on)
	-- (Artifacts says how many there are, as the old Keplorayx's did.)
	local n = M.artifact_count()
	for _, r in rows do
		if r.id == "artifacts" and n > 0 then
			r.note = tostring(n)
		end
	end
	return rows
end

function M.open(id: string): boolean
	return require("@app/kx/nav").open({ plugin = M.PLUGIN, id = id }, function() end)
end

function M.page(id: string): Node?
	if id == "buddies" then
		return require("@app/kx/buddies/page").board()
	elseif id == "buddy" then
		return require("@app/kx/buddies/page").buddy()
	elseif id == "account" then
		return require("@app/kx/synced").page()
	elseif id == "kanban" then
		return kanban()
	elseif id == "artifacts" then
		return artifacts()
	end
	return nil
end

return M
