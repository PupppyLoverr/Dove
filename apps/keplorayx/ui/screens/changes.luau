--!strict
-- The Changes panel: what the open thread has changed, against where it
-- started. Each file shows its +/−; open one for its hunks, set in the mono
-- face, added and removed lines washed in their colours. Read on a thread
-- of the host's own (kx.changes) whenever the panel opens or a turn ends.

local kx = require("@kx")
local store = require("@kx/store")
local nav = require("@screens/nav")

export type Line = { kind: string, old: number?, new: number?, text: string }
export type Hunk = { header: string, lines: { Line } }
export type File = { path: string, status: string, binary: boolean, added: number, removed: number, hunks: { Hunk } }

local M = {}

-- Kept across launches; a fixture's own, so one doesn't leak into the next.
M.open = if store.fixture ~= "" then ui.state("changes.open", false) else ui.persist("changes.open", false)

-- Development: a fixture may ask for the panel open.
local fx = (store :: any).fixture_data
if fx and fx.changes_open then
	M.open:set(true)
end

local files = ui.state("changes.files", {} :: { File })
local loading = ui.state("changes.loading", false)
local failure = ui.state("changes.failure", "")

function M.refresh()
	loading:set(true)
	failure:set("")
	store.changes(function(list: any)
		files:set(list or {})
		loading:set(false)
	end, function(why: any)
		failure:set(tostring(if type(why) == "table" then why.message else why))
		loading:set(false)
	end)
end

-- Read as it opens and as another thread opens…
local open_id = ui.computed(function(): string
	local t = store.thread:get()
	return if M.open:get() and t then t.id else ""
end)
ui.effect(function()
	if open_id:get() ~= "" then
		M.refresh()
	end
end)
-- …and again as a turn ends.
local running = ui.computed(function(): boolean
	local t = store.thread:get()
	return t ~= nil and t.running
end)
local was_running = false
ui.effect(function()
	local now = running:get()
	if was_running and not now and M.open:peek() then
		M.refresh()
	end
	was_running = now
end)

local totals = ui.computed(function()
	local a, r = 0, 0
	for _, f in files:get() do
		a += f.added
		r += f.removed
	end
	return { added = a, removed = r, count = #files:get() }
end)
-- (The panel's home says them beside the thread's branch.)
M.totals = totals

local LINE_BG: { [string]: string } = { add = "$kx.diff.add", del = "$kx.diff.del", ctx = "#00000000" }
local LINE_FG: { [string]: string } = { add = "$text.primary", del = "$text.primary", ctx = "$text.secondary" }
local SIGN: { [string]: string } = { add = "+", del = "−", ctx = " " }

local function line_row(l: Line): Node
	return ui.row {
		w_full = true,
		bg = LINE_BG[l.kind],
		gap = "$space.2",
		px = "$space.2",
		ui.text {
			if l.kind == "del" then (if l.old then tostring(l.old) else "") else (if l.new then tostring(l.new) else ""),
			literal = true,
			w = 32,
			shrink = 0,
			align = "right",
			tabular = true,
			font = "$font.mono",
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		},
		ui.text {
			SIGN[l.kind],
			literal = true,
			w = 10,
			shrink = 0,
			font = "$font.mono",
			text_size = "$font.size.xs",
			color = if l.kind == "add" then "$success" elseif l.kind == "del" then "$danger" else "$text.tertiary",
		},
		ui.text {
			l.text,
			literal = true,
			grow = 1,
			min_w = 0,
			truncate = true,
			selectable = true,
			font = "$font.mono",
			text_size = "$font.size.xs",
			line_height = "$font.line.xs",
			color = LINE_FG[l.kind],
		},
	}
end

local function file_row(f: File): Node
	local open = ui.use_state(false)
	local name = string.match(f.path, "([^/\\]+)$") or f.path
	local dir = string.sub(f.path, 1, #f.path - #name)
	local hunks: { Node } = {}
	for _, h in f.hunks do
		table.insert(hunks, ui.text {
			h.header,
			literal = true,
			px = "$space.2",
			py = "$space.1",
			font = "$font.mono",
			text_size = "$font.size.xs",
			color = "$text.tertiary",
			truncate = true,
		})
		for _, l in h.lines do
			table.insert(hunks, line_row(l))
		end
	end
	return ui.column {
		w_full = true,
		kx.Row {
			size = "sm",
			expanded = function()
				return open:get()
			end,
			on_click = function()
				open:set(not open:peek())
			end,
			ui.column {
				color = "$text.tertiary",
				kx.Icon {
					name = function()
						return if open:get() then "alt-arrow-down" else "alt-arrow-right"
					end,
					size = 12,
				},
			},
			ui.text { name, literal = true, truncate = true, shrink = 0, max_w = 220, weight = "$font.weight.medium" },
			ui.text { dir, literal = true, truncate = true, grow = 1, min_w = 0, text_size = "$font.size.xs", color = "$text.tertiary" },
			if f.status == "added" then kx.Hint { "new", literal = true } elseif f.status == "deleted" then kx.Hint { "deleted", literal = true } else nil,
			ui.text { `+{f.added}`, literal = true, tabular = true, text_size = "$font.size.xs", color = "$success" },
			ui.text { `−{f.removed}`, literal = true, tabular = true, text_size = "$font.size.xs", color = "$danger" },
		},
		ui.show(open, ui.column {
			w_full = true,
			my = "$space.1",
			py = "$space.1",
			rounded = "$radius.md",
			bg = "$surface.sunken",
			overflow = "hidden",
			if f.binary then kx.Caption { "A binary file", literal = true, p = "$space.2" } else ui.column { w_full = true, table.unpack(hunks) },
		}),
	}
end

-- The totals and Refresh, for the panel's header.
function M.summary(): Node
	return ui.row {
		gap = "$space.2",
		items = "center",
		ui.text {
			function()
				local t = totals:get()
				return if t.count == 0 then "" else `{t.count} {if t.count == 1 then "file" else "files"}`
			end,
			literal = true,
			text_size = "$font.size.sm",
			color = "$text.tertiary",
		},
		ui.text {
			function()
				return `+{totals:get().added}`
			end,
			literal = true,
			tabular = true,
			text_size = "$font.size.sm",
			color = "$success",
		},
		ui.text {
			function()
				return `−{totals:get().removed}`
			end,
			literal = true,
			tabular = true,
			text_size = "$font.size.sm",
			color = "$danger",
		},
		kx.Button {
			tone = "ghost",
			size = "xs",
			icon = true,
			label = "Refresh",
			busy = loading,
			on_click = M.refresh,
			kx.Icon { name = "refresh", size = 14 },
		},
	}
end

-- The files, each opening to its hunks.
function M.list(): Node
	return ui.scroll {
		grow = 1,
		min_h = 0,
		w_full = true,
		px = "$space.2",
		pb = "$space.4",
		ui.show(
			function()
				return failure:get() ~= ""
			end,
			kx.Empty {
				icon = "danger-triangle",
				title = "Couldn't read the changes",
				hint = failure:peek(),
			},
			ui.show(
				function()
					return #files:get() > 0
				end,
				ui.each(files, file_row, {
					key = function(f: File)
						return f.path
					end,
					w_full = true,
					gap = "$kx.space.hair",
				}),
				kx.Empty {
					icon = "file-diff",
					title = "No changes yet",
					hint = "What this thread changes shows here, file by file.",
				}
			)
		),
	}
end

return M
