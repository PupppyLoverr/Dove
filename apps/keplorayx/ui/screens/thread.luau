--!strict
-- A thread, as Zeron's transcript tells one (ds/transcript, activity,
-- user_message; MIT, see THIRD_PARTY_NOTICES.md): what you sent sits at
-- the right in a bubble that folds when long; what the agent did between
-- two replies gathers into one quiet line ("Worked · read 3 files · ran 2
-- commands · edited 1 file") that opens into its tools on a tree's line,
-- each with what it touched (+/−); the reply reads as a page at a
-- measure of about 68 characters, the part being written streaming in
-- below. While the agent works, the ring turns under the last beat. When
-- it needs you, a card waits above the composer, lit in the warm colour,
-- and nowhere else is.

local kx = require("@kx")
local Status = require("@kx/status")
local store = require("@kx/store")
local ray = require("@screens/ray")
local changes = require("@screens/changes")
local panel = require("@screens/panel")
local nav = require("@screens/nav")

type Entry = store.Entry

local M = {}

-- ── The rows ───────────────────────────────────────────────────────────
-- Entries as the transcript shows them: a run of tools and thinking
-- between two replies is one `activity` row (keyed by its first entry, so
-- it keeps its node as the run grows), and the block being written is one
-- more row at the end (its text read live by that row alone).

type Row = { id: string, kind: string, text: string, tool: store.Tool?, steps: { Entry }? }

local draft_id = ui.computed(function()
	return store.draft:get().id
end)

local function is_step(e: Entry): boolean
	return e.kind == "tool" or e.kind == "thinking"
end

-- Each activity row's steps, by its key: read by that row alone.
local steps_of = ui.computed(function(): { [string]: { Entry } }
	local out: { [string]: { Entry } } = {}
	local key: string? = nil
	for _, e in store.entries:get() do
		if is_step(e) then
			if not key then
				key = "act:" .. e.id
				out[key] = {}
			end
			table.insert(out[key :: string], e)
		else
			key = nil
		end
	end
	return out
end)

local rows = ui.computed(function(): { Row }
	local out: { Row } = {}
	local open = false
	for _, e in store.entries:get() do
		if is_step(e) then
			if not open then
				table.insert(out, { id = "act:" .. e.id, kind = "activity", text = "" })
				open = true
			end
		else
			open = false
			table.insert(out, { id = e.id, kind = e.kind, text = e.text, tool = e.tool })
		end
	end
	local id = draft_id:get()
	if id ~= "" then
		table.insert(out, { id = "draft:" .. id, kind = "draft", text = "" })
	end
	return out
end)

local function measure(child: Node): Node
	return ui.row {
		w_full = true,
		justify = "center",
		px = "$space.6",
		ui.column { w_full = true, max_w = kx.MEASURE, child },
	}
end

-- ── What you sent ──────────────────────────────────────────────────────

local FOLD_AT = 600

local function user_beat(e: Row): Node
	local long = #e.text > FOLD_AT or select(2, string.gsub(e.text, "\n", "")) > 10
	local open = ui.use_state(false)
	return ui.row {
		w_full = true,
		justify = "end",
		pt = "$space.5",
		pb = "$space.2",
		ui.column {
			max_w = "82%",
			gap = "$space.1",
			px = "$space.3",
			py = "$space.2",
			rounded = "$radius.xl",
			bg = "$surface.raised",
			inset_shadow = "$kx.lip.inset",
			ui.text {
				function()
					if long and not open:get() then
						return string.sub(e.text, 1, FOLD_AT) .. "…"
					end
					return e.text
				end,
				literal = true,
				selectable = true,
				wrap = true,
				text_size = "$font.size.lg",
				line_height = "$font.line.lg",
			},
			if long
				then ui.button {
					h = 22,
					items = "center",
					gap = "$space.1",
					color = "$text.tertiary",
					hover = { color = "$text.primary" },
					cursor = "pointer",
					on_click = function()
						open:set(not open:peek())
					end,
					ui.text {
						function()
							return if open:get() then "Show less" else "Show all"
						end,
						literal = true,
						text_size = "$font.size.sm",
					},
				}
				else nil,
		},
	}
end

-- ── What the agent did ─────────────────────────────────────────────────

local TOOL_ICON: { [string]: string } = {
	Read = "document",
	Edit = "pen",
	Write = "document-add",
	MultiEdit = "pen",
	Bash = "terminal",
	Grep = "magnifer",
	Glob = "folder-tree",
	WebFetch = "global",
	WebSearch = "globe",
	Task = "sparkle-agent",
}

local function file_chip(c: store.Change): Node
	local name = string.match(c.path, "([^/\\]+)$") or c.path
	return ui.row {
		h = 22,
		px = "$space.2",
		gap = "$space.1",
		items = "center",
		rounded = "$radius.md",
		bg = "$kx.secondary.bg",
		tooltip = c.path,
		ui.text { name, literal = true, font = "$font.mono", text_size = "$font.size.xs", color = "$text.secondary" },
		ui.text { `+{c.added}`, literal = true, tabular = true, text_size = "$font.size.xs", color = "$kx.syntax.inserted" },
		ui.text { `−{c.removed}`, literal = true, tabular = true, text_size = "$font.size.xs", color = "$kx.syntax.deleted" },
	}
end

-- The summary line's words: "read 3 files · ran 2 commands · edited a file".
local function summary(steps: { Entry }): string
	local read, ran, edited, searched, other, failed = 0, 0, 0, 0, 0, 0
	for _, e in steps do
		local t = e.tool
		if t then
			local n = t.name
			if n == "Read" then
				read += 1
			elseif n == "Bash" then
				ran += 1
			elseif n == "Edit" or n == "Write" or n == "MultiEdit" then
				edited += math.max(1, #t.changes)
			elseif n == "Grep" or n == "Glob" or n == "WebSearch" or n == "WebFetch" then
				searched += 1
			else
				other += 1
			end
			if t.state == "failed" then
				failed += 1
			end
		end
	end
	local parts: { string } = {}
	local function say(n: number, one: string, many: string)
		if n == 1 then
			table.insert(parts, one)
		elseif n > 1 then
			table.insert(parts, string.format(many, n))
		end
	end
	say(read, "read a file", "read %d files")
	say(ran, "ran a command", "ran %d commands")
	say(edited, "edited a file", "edited %d files")
	say(searched, "searched once", "searched %d times")
	say(other, "used a tool", "used %d tools")
	say(failed, "1 failed", "%d failed")
	if #parts == 0 then
		return "Thought it through"
	end
	local first = parts[1]
	parts[1] = string.upper(string.sub(first, 1, 1)) .. string.sub(first, 2)
	return table.concat(parts, " · ")
end

local STEP_H = 28
local TRUNK, TIP = 8, 22

local function step_view(e: Entry, last: () -> boolean): Node
	local tool = e.tool
	local state = if tool then tool.state else "done"
	local chips: { Node } = {}
	if tool then
		for _, c in tool.changes do
			table.insert(chips, file_chip(c))
		end
	end
	local body = ui.column {
		w_full = true,
		pl = ui.raw(TIP + 4, "clears the tree's arm"),
		gap = "$space.1",
		ui.row {
			w_full = true,
			h = STEP_H,
			gap = "$space.2",
			items = "center",
			ui.column {
				color = if state == "failed" then "$danger" else "$text.tertiary",
				if state == "running"
					then kx.StatusIcon { kind = "working", size = 14 }
					else kx.Icon { name = if tool then TOOL_ICON[tool.name] or "widget" else "sparkle", size = 14 },
			},
			ui.text {
				e.text,
				literal = true,
				truncate = true,
				grow = 1,
				min_w = 0,
				italic = e.kind == "thinking",
				text_size = "$font.size.md",
				color = if state == "running" then "$text.primary" else "$text.secondary",
			},
			if state == "failed" then kx.Hint { "failed", literal = true, color = "$danger" } else nil,
		},
		if #chips > 0 then ui.row { gap = "$space.1", flex_wrap = true, pb = "$space.1", table.unpack(chips) } else nil,
	}
	return ui.div {
		relative = true,
		w_full = true,
		body,
		ui.canvas {
			absolute = true,
			left = 0,
			top = 0,
			bottom = 0,
			w = TIP,
			pointer_events = "none",
			draw = function(c: any, _w: number, h: number)
				c:fill(
					kx.tree_line.branch { x = TRUNK, mid = STEP_H / 2, tip = TIP - 4, h = h, bend = 5, on = not last() },
					"$kx.tree"
				)
			end,
		},
	}
end

local function activity_beat(r: Row): Node
	local open = ui.use_state(false)
	local function steps(): { Entry }
		return steps_of:get()[r.id] or {}
	end
	local function working(): boolean
		for _, e in steps() do
			if e.tool and e.tool.state == "running" then
				return true
			end
		end
		return false
	end
	return ui.column {
		w_full = true,
		pt = "$space.2",
		ui.button {
			h = 28,
			items = "center",
			gap = "$space.2",
			cursor = "pointer",
			bg = "#00000000",
			color = "$text.tertiary",
			hover = { bg = "#00000000", color = "$text.primary" },
			active = { bg = "#00000000" },
			expanded = function()
				return open:get()
			end,
			on_click = function()
				open:set(not open:peek())
			end,
			ui.text {
				function()
					return summary(steps())
				end,
				literal = true,
				text_size = "$font.size.md",
			},
			ui.column {
				rotate = function(): number
					return if open:get() then 0 else -90
				end,
				kx.Icon { name = "chevron-down", size = 12 },
			},
		},
		ui.show(
			function()
				return open:get() or working()
			end,
			ui.each(ui.computed(steps), function(e: Entry)
				return step_view(e, function()
					local list = steps()
					return list[#list].id == e.id
				end)
			end, {
				key = function(e: Entry)
					return e.id
				end,
				w_full = true,
				enter = { opacity = { 0, 1 }, duration = 160 },
			})
		),
	}
end

-- ── A row ──────────────────────────────────────────────────────────────

local function entry_row(e: Row): Node
	local body: Node
	if e.kind == "user" then
		body = user_beat(e)
	elseif e.kind == "activity" then
		body = activity_beat(e)
	elseif e.kind == "draft" then
		body = ui.column {
			w_full = true,
			pt = "$space.2",
			ui.markdown_view {
				function()
					return store.draft:get().text
				end,
				w_full = true,
				selectable = true,
				text_size = "$font.size.lg",
				line_height = "$font.line.xl",
			},
		}
	elseif e.kind == "error" then
		body = ui.row {
			w_full = true,
			gap = "$space.2",
			py = "$space.2",
			px = "$space.3",
			rounded = "$radius.lg",
			bg = "$kx.danger.soft",
			color = "$danger",
			kx.Icon { name = "danger-triangle", size = 16 },
			kx.Body { e.text, literal = true, color = "$danger", wrap = true },
		}
	elseif e.kind == "notice" then
		body = kx.Caption { e.text, literal = true, wrap = true, py = "$space.1" }
	else
		body = ui.column {
			w_full = true,
			pt = "$space.2",
			ui.markdown_view {
				e.text,
				w_full = true,
				selectable = true,
				text_size = "$font.size.lg",
				line_height = "$font.line.xl",
			},
		}
	end
	return measure(body)
end

-- ── What the agent waits on ────────────────────────────────────────────

local function approval_card(): Node
	local function get(): store.Approval?
		return store.approval:get()
	end
	local function decide(decision: string)
		local a = store.approval:peek()
		if a then
			store.decide(a.request_id, decision)
		end
	end
	return ui.column {
		w_full = true,
		gap = "$space.3",
		p = "$space.4",
		rounded = "$radius.xl",
		bg = "$surface.overlay",
		border = 1,
		border_color = "$kx.attention.soft",
		shadow = "$shadow.lg",
		live = "polite",
		ui.row {
			gap = "$space.2",
			items = "center",
			ui.column { color = "$kx.attention", kx.Icon { name = "status-needs-you", size = 16 } },
			ui.text {
				function()
					local t = store.thread:get()
					local a = get()
					local who = if t then t.harness else "The agent"
					return if a and #a.questions > 0 then `{who} has a question` else `{who} wants to run {if a then a.tool else "a tool"}`
				end,
				literal = true,
				weight = "$font.weight.medium",
				text_size = "$font.size.lg",
			},
		},
		ui.text {
			function()
				local a = get()
				return if a then a.summary else ""
			end,
			literal = true,
			wrap = true,
			selectable = true,
			font = "$font.mono",
			text_size = "$font.size.sm",
			line_height = "$font.line.sm",
			color = "$kx.syntax.text",
			p = "$space.3",
			rounded = "$radius.lg",
			bg = "$surface.sunken",
			w_full = true,
			max_h = 220,
		},
		ui.row {
			w_full = true,
			gap = "$space.2",
			items = "center",
			kx.Hint { "Y allows · N denies", literal = true, grow = 1 },
			kx.Button { tone = "ghost", size = "sm", on_click = function()
				decide("deny")
			end, "Deny" },
			kx.Button { tone = "secondary", size = "sm", on_click = function()
				decide("allow_always")
			end, "Always allow" },
			kx.Button { tone = "primary", size = "sm", on_click = function()
				decide("allow_once")
			end, "Allow" },
		},
	}
end

-- ── The page ───────────────────────────────────────────────────────────

local function thread_kind(): Status.Kind
	local t = store.thread:get()
	if not t then
		return "idle"
	end
	if store.approval:get() then
		return "approval"
	end
	return if t.running then "working" else "done"
end

local function panel_button(tab: string, icon: string, label: string): Node
	return kx.Button {
		tone = "ghost",
		size = "sm",
		icon = true,
		label = label,
		tooltip = label,
		on_click = function()
			panel.toggle(tab)
		end,
		kx.Icon {
			name = icon,
			size = 16,
			color = function()
				return if changes.open:get() and panel.tab:get() == tab then "$text.primary" else "$text.tertiary"
			end,
		},
	}
end

local header = ui.row {
	h = "$kx.titlebar",
	w_full = true,
	window_drag = true,
	items = "center",
	px = "$space.4",
	gap = "$space.2",
	pr = function()
		return if changes.open:get() then "$space.3" else nav.caption_inset()
	end,
	ui.show(function()
		return not nav.sidebar_open:get()
	end, kx.Button {
		tone = "ghost",
		size = "sm",
		icon = true,
		label = "Show the sidebar (Ctrl+B)",
		tooltip = "Show the sidebar  Ctrl+B",
		on_click = function()
			nav.sidebar_open:set(true)
		end,
		kx.Icon { name = "sidebar", size = 16 },
	}),
	kx.StatusIcon { kind = thread_kind, size = 15 },
	ui.text {
		function()
			local t = store.thread:get()
			return if t then t.title else ""
		end,
		literal = true,
		truncate = true,
		min_w = 0,
		weight = "$font.weight.medium",
	},
	ui.text {
		function()
			local t = store.thread:get()
			if not t then
				return ""
			end
			return if t.running and t.status ~= "" then `{t.harness} · {t.status}` else t.harness
		end,
		literal = true,
		truncate = true,
		min_w = 0,
		shrink = 2,
		max_w = 320,
		text_size = "$font.size.sm",
		color = "$text.tertiary",
	},
	ui.div { grow = 1 },
	panel_button("files", "folder-with-files", "Files  Ctrl+P"),
	panel_button("browser", "global", "Browser  Ctrl+Shift+B"),
	panel_button("terminal", "terminal", "Terminal  Ctrl+`"),
	panel_button("changes", "file-diff", "Changes  Ctrl+J"),
}

-- When the turn now running began, by the clock (for the ring's time).
local turn_began = ui.state("thread.turn_began", 0)
ui.effect(function()
	local t = store.thread:get()
	local running = t ~= nil and t.running
	if running and turn_began:peek() == 0 then
		turn_began:set(os.time())
	elseif not running and turn_began:peek() ~= 0 then
		turn_began:set(0)
	end
end)

-- (The panel beside it is the window's: app.luau.)
function M.view(): Node
	return M.column()
end

function M.column(): Node
	return ui.column {
		grow = 1,
		min_w = 0,
		h_full = true,
		-- It gives way to the panel beside it: what it holds wraps or
		-- truncates, never pushes the window wider.
		overflow = "hidden",
		header,
		ui.list {
			data = rows,
			key = function(e: Row)
				return e.id
			end,
			row = entry_row,
			measured = true,
			follow = true,
			height = 48,
			w_full = true,
			grow = 1,
			min_h = 0,
			pb = "$space.4",
			scrollbar = true,
			overscroll = "contain",
			fade = 24,
		},
		ui.column {
			w_full = true,
			items = "center",
			px = "$space.6",
			pb = "$space.4",
			gap = "$space.2",
			ui.column {
				w_full = true,
				max_w = kx.MEASURE + 40,
				gap = "$space.2",
				ui.show(function()
					return turn_began:get() > 0 and store.approval:get() == nil
				end, ui.row {
					w_full = true,
					px = "$space.2",
					kx.Working {
						since = function()
							return turn_began:get()
						end,
						label = "Working",
					},
				}),
				ui.presence(function()
					return if store.approval:get() then "waiting" else nil
				end, function()
					return approval_card()
				end, {
					w_full = true,
					enter = { opacity = { 0, 1 }, y = { 12, 0 }, duration = 220, ease = kx.motion.ENTER },
					exit = { opacity = { 1, 0 }, duration = 120 },
				}),
				ray.Ray { place = "thread" },
			},
		},
	}
end

return M
