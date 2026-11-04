--!strict
-- A thread, told as beats. Your prompt opens a beat: a quiet rule beside
-- it, no bubble. The agent's thinking is one dim line you can open. Its
-- tools run down a spine, each with what it touched (+/−). The reply reads
-- as a page, at a measure of about 68 characters, and the part being
-- written streams in below it. When the agent needs you, a card waits
-- above the composer, lit in the warm colour, and nowhere else is.

local kx = require("@kx")
local store = require("@kx/store")
local ray = require("@screens/ray")
local changes = require("@screens/changes")
local nav = require("@screens/nav")

type Entry = store.Entry

local M = {}

-- The entries, and the block being written as one more row at the end (its
-- text read live by that row alone, so a streamed word writes no list).
local draft_id = ui.computed(function()
	return store.draft:get().id
end)
local rows = ui.computed(function(): { Entry }
	local entries = store.entries:get()
	local id = draft_id:get()
	if id == "" then
		return entries
	end
	local all = table.clone(entries)
	table.insert(all, { id = "draft:" .. id, kind = "draft", text = "" })
	return all
end)

local function measure(child: Node): Node
	return ui.row {
		w_full = true,
		justify = "center",
		px = "$space.6",
		ui.column { w_full = true, max_w = kx.MEASURE, child },
	}
end

local function file_chip(c: store.Change): Node
	local name = string.match(c.path, "([^/\\]+)$") or c.path
	return ui.row {
		h = 22,
		px = "$space.2",
		gap = "$space.1",
		items = "center",
		rounded = "$radius.sm",
		bg = "$surface.base",
		border = 1,
		border_color = "$border.subtle",
		tooltip = c.path,
		ui.text { name, literal = true, font = "$font.mono", text_size = "$font.size.xs", color = "$text.secondary" },
		ui.text { `+{c.added}`, literal = true, tabular = true, text_size = "$font.size.xs", color = "$success" },
		ui.text { `−{c.removed}`, literal = true, tabular = true, text_size = "$font.size.xs", color = "$danger" },
	}
end

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

local function tool_beat(e: Entry): Node
	local tool = e.tool :: store.Tool
	local state = tool.state
	local chips: { Node } = {}
	for _, c in tool.changes do
		table.insert(chips, file_chip(c))
	end
	local running = state == "running"
	local glow = if running then kx.motion.breath(0.4, 1) else nil
	return ui.row {
		w_full = true,
		gap = "$space.3",
		items = "stretch",
		-- The spine: a hairline down the beat, a node where this tool sits.
		ui.column {
			w = 16,
			shrink = 0,
			items = "center",
			ui.div { w = 1, h = 8, bg = "$border" },
			ui.column {
				color = if state == "failed" then "$danger" elseif running then "$accent" else "$text.tertiary",
				opacity = if glow then glow else 1,
				kx.Icon { name = TOOL_ICON[tool.name] or "widget", size = 14 },
			},
			ui.div { w = 1, grow = 1, min_h = 8, bg = "$border" },
		},
		ui.column {
			grow = 1,
			min_w = 0,
			py = "$space.1",
			gap = "$space.1",
			ui.row {
				w_full = true,
				gap = "$space.2",
				items = "center",
				ui.text {
					e.text,
					literal = true,
					truncate = true,
					grow = 1,
					min_w = 0,
					text_size = "$font.size.sm",
					color = if running then "$text.primary" else "$text.secondary",
				},
				if state == "failed" then kx.Hint { "failed", literal = true, color = "$danger" } else nil,
			},
			if #chips > 0 then ui.row { gap = "$space.1", flex_wrap = true, table.unpack(chips) } else nil,
		},
	}
end

local function thinking_beat(e: Entry): Node
	local open = ui.use_state(false)
	return ui.button {
		w_full = true,
		p = 0,
		bg = "#00000000",
		hover = { bg = "#00000000" },
		items = "start",
		gap = "$space.2",
		cursor = "pointer",
		expanded = function()
			return open:get()
		end,
		on_click = function()
			open:set(not open:peek())
		end,
		ui.column { w = 16, items = "center", pt = "$kx.space.hair", color = "$text.tertiary", kx.Icon { name = "sparkle", size = 14 } },
		ui.text {
			e.text,
			literal = true,
			grow = 1,
			min_w = 0,
			italic = true,
			wrap = function()
				return open:get()
			end,
			truncate = function()
				return not open:get()
			end,
			text_size = "$font.size.sm",
			line_height = "$font.line.sm",
			color = "$text.tertiary",
		},
	}
end

local function user_beat(e: Entry): Node
	return ui.row {
		w_full = true,
		gap = "$space.3",
		pt = "$space.5",
		ui.div { w = 2, rounded = "$radius.full", bg = "$accent", opacity = 0.7, shrink = 0 },
		ui.column {
			grow = 1,
			min_w = 0,
			gap = "$space.1",
			kx.Hint { "You", literal = true },
			kx.Body { e.text, literal = true, selectable = true, weight = "$font.weight.medium" },
		},
	}
end

local function entry_row(e: Entry): Node
	local body: Node
	if e.kind == "user" then
		body = user_beat(e)
	elseif e.kind == "tool" then
		body = tool_beat(e)
	elseif e.kind == "thinking" then
		body = ui.column { w_full = true, pt = "$space.1", thinking_beat(e) }
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
			},
		}
	elseif e.kind == "error" then
		body = ui.row {
			w_full = true,
			gap = "$space.2",
			py = "$space.2",
			color = "$danger",
			kx.Icon { name = "danger-triangle", size = 16 },
			kx.Body { e.text, literal = true, color = "$danger" },
		}
	elseif e.kind == "notice" then
		body = kx.Caption { e.text, literal = true, wrap = true, py = "$space.1" }
	else
		body = ui.column {
			w_full = true,
			pt = "$space.2",
			ui.markdown_view { e.text, w_full = true, selectable = true },
		}
	end
	return measure(body)
end

-- The card for what the agent waits on. One warm thing on the screen.
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
		rounded = "$kx.card.radius",
		bg = "$surface.raised",
		border = 1,
		border_color = "$kx.attention.soft",
		inset_shadow = "$kx.lip.inset_strong",
		shadow = "$shadow.md",
		live = "polite",
		ui.row {
			gap = "$space.2",
			items = "center",
			ui.div { w = 8, h = 8, rounded = "$radius.full", bg = "$kx.attention" },
			ui.text {
				function()
					local t = store.thread:get()
					local a = get()
					local who = if t then t.harness else "The agent"
					return if a and #a.questions > 0 then `{who} has a question` else `{who} wants to run {if a then a.tool else "a tool"}`
				end,
				literal = true,
				weight = "$font.weight.semibold",
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
			p = "$space.3",
			rounded = "$radius.md",
			bg = "$surface.sunken",
			w_full = true,
		},
		ui.row {
			w_full = true,
			gap = "$space.2",
			items = "center",
			kx.Hint { "Y to allow · N to deny", literal = true, grow = 1 },
			kx.Button { tone = "ghost", size = "sm", on_click = function()
				decide("deny")
			end, "Deny" },
			kx.Button { tone = "secondary", size = "sm", on_click = function()
				decide("allow_always")
			end, "Always allow" },
			kx.Button { tone = "attention", size = "sm", on_click = function()
				decide("allow_once")
			end, "Allow" },
		},
	}
end

local header = ui.row {
	h = "$kx.titlebar",
	w_full = true,
	window_drag = true,
	items = "center",
	px = "$space.5",
	gap = "$space.3",
	-- Under the caption buttons only while no panel stands to its right.
	pr = function()
		return if changes.open:get() then "$space.3" else nav.caption_inset()
	end,
	ui.text {
		function()
			local t = store.thread:get()
			return if t then t.title else ""
		end,
		literal = true,
		truncate = true,
		min_w = 0,
		weight = "$font.weight.semibold",
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
		shrink = 0,
		max_w = 320,
		text_size = "$font.size.sm",
		color = "$text.tertiary",
	},
	ui.div { grow = 1 },
	kx.Tooltip(
		kx.Button {
			tone = "ghost",
			size = "sm",
			icon = true,
			label = "Changes",
			on_click = ui.toggle(changes.open),
			kx.Icon {
				name = "file-diff",
				size = 16,
				color = function()
					return if changes.open:get() then "$accent" else "$text.secondary"
				end,
			},
		},
		"Changes  Ctrl J"
	),
}

function M.view(): Node
	return ui.row {
		grow = 1,
		min_w = 0,
		h_full = true,
		M.column(),
		ui.presence(function()
			return if changes.open:get() then "open" else nil
		end, function()
			return changes.view()
		end, {
			h_full = true,
			enter = { opacity = { 0, 1 }, x = { 24, 0 }, duration = 240, ease = kx.motion.ENTER },
			exit = { opacity = { 1, 0 }, x = { 0, 16 }, duration = 140 },
		}),
	}
end

function M.column(): Node
	return ui.column {
		grow = 1,
		min_w = 0,
		h_full = true,
		header,
		ui.list {
			data = rows,
			key = function(e: Entry)
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
			pb = "$space.5",
			gap = "$space.3",
			ui.column {
				w_full = true,
				max_w = kx.MEASURE,
				gap = "$space.3",
				ui.presence(function()
					return if store.approval:get() then "waiting" else nil
				end, function()
					return approval_card()
				end, {
					w_full = true,
					enter = { opacity = { 0, 1 }, y = { 12, 0 }, duration = 260, ease = kx.motion.ENTER },
					exit = { opacity = { 1, 0 }, duration = 140 },
				}),
				ray.Ray { place = "thread" },
			},
		},
	}
end

return M
