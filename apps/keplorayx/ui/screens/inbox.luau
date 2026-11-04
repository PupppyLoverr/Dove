--!strict
-- The Attention inbox: one queue of what needs you (an agent waiting on an
-- answer) and what just finished, newest first. J and K move, Enter opens.

local kx = require("@kx")
local store = require("@kx/store")
local nav = require("@screens/nav")

local M = {}

local queue = ui.computed(function(): { store.Thread }
	local waiting, done = {}, {}
	for _, t in store.threads:get() do
		if t.status == "needs_you" then
			table.insert(waiting, t)
		elseif t.unread then
			table.insert(done, t)
		end
	end
	for _, t in done do
		table.insert(waiting, t)
	end
	return waiting
end)

local at = ui.state("inbox.at", 1)

local function open(t: store.Thread)
	nav.go("thread")
	store.open(t.id)
end

local function row(t: store.Thread, i: number): Node
	local function get()
		return t
	end
	return kx.Row {
		size = "lg",
		selected = function()
			return at:get() == i
		end,
		on_click = function()
			at:set(i)
			open(get())
		end,
		kx.AgentMark {
			agent = get().harness_id,
			needs_you = function()
				return get().status == "needs_you"
			end,
		},
		ui.column {
			grow = 1,
			min_w = 0,
			gap = "$kx.space.hair",
			ui.text {
				function()
					return get().title
				end,
				literal = true,
				truncate = true,
				weight = "$font.weight.medium",
			},
			ui.text {
				function()
					local t = get()
					return if t.status == "needs_you" then `{t.harness} is waiting for you · {t.project}` else `{t.harness} finished · {t.project}`
				end,
				literal = true,
				truncate = true,
				text_size = "$font.size.sm",
				color = function()
					return if get().status == "needs_you" then "$kx.attention" else "$text.tertiary"
				end,
			},
		},
		kx.Icon { name = "alt-arrow-right", size = 14, color = "$text.tertiary" },
	}
end

function M.view(): Node
	return ui.column {
		grow = 1,
		min_w = 0,
		h_full = true,
		focusable = true,
		on_key = function(e: any)
			local n = #queue:peek()
			if n == 0 then
				return nil
			end
			if e.key == "j" or e.key == "ArrowDown" then
				at:set(math.min(n, at:peek() + 1))
				return true
			elseif e.key == "k" or e.key == "ArrowUp" then
				at:set(math.max(1, at:peek() - 1))
				return true
			elseif e.key == "Enter" then
				open(queue:peek()[at:peek()])
				return true
			end
			return nil
		end,
		ui.row {
			h = "$kx.titlebar",
			w_full = true,
			window_drag = true,
			items = "center",
			px = "$space.5",
			kx.Label { "Inbox", literal = true },
		},
		ui.column {
			w_full = true,
			grow = 1,
			min_h = 0,
			items = "center",
			px = "$space.6",
			ui.column {
				w_full = true,
				max_w = kx.MEASURE,
				gap = "$kx.space.hair",
				ui.show(
					function()
						return #queue:get() > 0
					end,
					ui.each(queue, row, {
						key = function(t: store.Thread)
							return t.id
						end,
						w_full = true,
						gap = "$kx.space.hair",
						animate = { duration = 260, ease = kx.motion.EASE },
					}),
					kx.Empty {
						icon = "inbox",
						title = "Nothing needs you",
						hint = "When an agent asks something or finishes, it waits here.",
					}
				),
			},
		},
	}
end

return M
