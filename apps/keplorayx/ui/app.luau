--!strict
-- Keplorayx's main window. Phase A's proof: threads, a transcript that
-- streams, a composer. The look is a placeholder on zgx's standard tokens
-- until the Orbit design system (ui/kx) lands from Paper.

local store = require("@kx/store")
type Entry = store.Entry

ui.theme.strict("warn")

local TITLE_H = 44
local SIDEBAR_W = 264

-- ── Sidebar ────────────────────────────────────────────────────────────

local function thread_row(t: store.Thread, _i: number, row: any)
	return ui.button {
		variant = "ghost",
		w_full = true,
		justify = "start",
		px = "$space.3",
		py = "$space.2",
		rounded = "$radius.md",
		bg = function()
			local open = store.thread:get()
			return if open and open.id == row:get().id then "$selected" else "transparent"
		end,
		on_click = function()
			store.open(row:get().id)
		end,
		ui.column {
			grow = 1,
			gap = 2,
			items = "start",
			ui.text {
				function()
					return row:get().title
				end,
				literal = true,
				truncate = true,
				w_full = true,
			},
			ui.text {
				function()
					local r = row:get()
					return `{r.project} · {r.status}`
				end,
				literal = true,
				truncate = true,
				w_full = true,
				color = "$text.tertiary",
				text_size = "$font.size.sm",
			},
		},
	}
end

local sidebar = ui.column {
	w = SIDEBAR_W,
	h_full = true,
	bg = "$surface.sunken",
	ui.row {
		h = TITLE_H,
		w_full = true,
		window_drag = true,
		items = "center",
		px = "$space.4",
		ui.text { "Keplorayx", weight = 600 },
	},
	ui.row {
		px = "$space.3",
		pb = "$space.2",
		ui.button {
			variant = "ghost",
			w_full = true,
			justify = "start",
			on_click = store.home,
			"New thread",
		},
	},
	ui.list {
		data = store.threads,
		key = function(t: store.Thread)
			return t.id
		end,
		height = 52,
		w_full = true,
		grow = 1,
		px = "$space.2",
		row = thread_row,
	},
}

-- ── Transcript ─────────────────────────────────────────────────────────

-- The entries, and the block being written as one more row at the end (its
-- text read live by that row alone, so a streamed word writes no list).
local draft_id = ui.computed(function()
	return store.draft:get().id
end)
local rows = ui.computed(function()
	local entries = store.entries:get()
	local id = draft_id:get()
	if id == "" then
		return entries
	end
	local all = table.clone(entries)
	table.insert(all, { id = "draft:" .. id, kind = "draft", text = "" })
	return all
end)

local function tool_row(e: Entry)
	local tool = e.tool :: store.Tool
	return ui.column {
		w_full = true,
		gap = 4,
		px = "$space.3",
		py = "$space.2",
		rounded = "$radius.md",
		border = 1,
		border_color = "$border.subtle",
		ui.row {
			gap = "$space.2",
			ui.text { tool.name, literal = true, weight = 600, text_size = "$font.size.sm" },
			ui.text { e.text, literal = true, truncate = true, grow = 1, color = "$text.secondary", text_size = "$font.size.sm" },
			ui.text { tool.state, literal = true, color = "$text.tertiary", text_size = "$font.size.sm" },
		},
	}
end

local function entry_row(e: Entry)
	local body
	if e.kind == "user" then
		body = ui.row {
			w_full = true,
			justify = "end",
			ui.column {
				max_w = "80%",
				px = "$space.3",
				py = "$space.2",
				rounded = "$radius.lg",
				bg = "$surface.raised",
				ui.text { e.text, literal = true, wrap = true, selectable = true },
			},
		}
	elseif e.kind == "tool" then
		body = tool_row(e)
	elseif e.kind == "draft" then
		body = ui.markdown_view {
			function()
				return store.draft:get().text
			end,
			w_full = true,
		}
	elseif e.kind == "thinking" then
		body = ui.text { e.text, literal = true, wrap = true, color = "$text.tertiary", italic = true }
	elseif e.kind == "error" then
		body = ui.text { e.text, literal = true, wrap = true, color = "$danger" }
	elseif e.kind == "notice" then
		body = ui.text { e.text, literal = true, wrap = true, color = "$text.secondary", text_size = "$font.size.sm" }
	else
		body = ui.markdown_view { e.text, w_full = true }
	end
	return ui.column { w_full = true, py = "$space.2", body }
end

local transcript = ui.list {
	data = rows,
	key = function(e: Entry)
		return e.id
	end,
	row = entry_row,
	measured = true,
	follow = true,
	height = 64,
	w_full = true,
	grow = 1,
	px = "$space.6",
	scrollbar = true,
	overscroll = "contain",
}

-- ── Composer ───────────────────────────────────────────────────────────

local message = ui.state("message", "")
local sending = ui.state("sending", false)
local failure = ui.state("failure", "")

local function send()
	local text = message:peek()
	if text:match("^%s*$") or sending:peek() then
		return
	end
	sending:set(true)
	failure:set("")
	store.send(text):after(function()
		message:set("")
		sending:set(false)
	end, function(why: string)
		failure:set(why)
		sending:set(false)
	end)
end

local composer = ui.column {
	w_full = true,
	px = "$space.6",
	pb = "$space.4",
	gap = "$space.2",
	ui.show(function()
		return failure:get() ~= ""
	end, ui.text { "{failure}", color = "$danger", text_size = "$font.size.sm" }),
	ui.row {
		w_full = true,
		gap = "$space.2",
		items = "end",
		p = "$space.2",
		rounded = "$radius.lg",
		bg = "$surface.raised",
		ui.textarea {
			value = message,
			label = "Message",
			placeholder = "Ask an agent…",
			wrap = true,
			grow = 1,
			h = 72,
			bg = "transparent",
			on_key = function(e)
				if e.key == "Enter" and not e.shift then
					send()
					return true
				end
				return nil
			end,
		},
		ui.show(function()
			local t = store.thread:get()
			return t ~= nil and t.running
		end, ui.button { on_click = store.interrupt, "Stop" }, ui.button {
			variant = "primary",
			disabled = function()
				return sending:get()
			end,
			on_click = send,
			"Send",
		}),
	},
}

-- ── The window ─────────────────────────────────────────────────────────

local title_bar = ui.row {
	h = TITLE_H,
	w_full = true,
	window_drag = true,
	items = "center",
	px = "$space.4",
	pr = function()
		local area, buttons = ui.window.titlebar_area:get(), ui.window.controls:get()
		local inset = if area and buttons and buttons.x >= area.x + area.width then buttons.width + 12 else 12
		return ui.raw(inset, "clears the window's caption buttons, which the system places")
	end,
	ui.text {
		function()
			local t = store.thread:get()
			return if t then t.title else "New thread"
		end,
		literal = true,
		truncate = true,
		weight = 600,
	},
	ui.text {
		function()
			local t = store.thread:get()
			return if t and t.status ~= "" then "  " .. t.status else ""
		end,
		literal = true,
		color = "$text.tertiary",
		text_size = "$font.size.sm",
	},
}

return ui.row {
	w_full = true,
	h_full = true,
	sidebar,
	ui.column {
		grow = 1,
		h_full = true,
		bg = "$surface",
		title_bar,
		transcript,
		composer,
	},
}
