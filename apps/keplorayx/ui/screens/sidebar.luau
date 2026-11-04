--!strict
-- The sidebar: glass, quiet. The way in (New thread, Inbox, Search), then
-- every thread under its project, each led by its agent's star: still
-- when the agent rests, breathing while it works, warm only when it needs
-- you. Settings and where your data lives sit at the foot.

local kx = require("@kx")
local store = require("@kx/store")
local nav = require("@screens/nav")

type Thread = store.Thread

local M = {}

-- "2m", "3h", "Tue", "4 Nov": as short as it can say it.
local function ago(ms: number): string
	local now = if store.fixture ~= "" then 1762300000000 else os.time() * 1000
	local s = math.max(0, (now - ms) / 1000)
	if s < 60 then
		return "now"
	elseif s < 3600 then
		return `{math.floor(s / 60)}m`
	elseif s < 86400 then
		return `{math.floor(s / 3600)}h`
	elseif s < 7 * 86400 then
		return `{math.floor(s / 86400)}d`
	end
	return os.date("%d %b", math.floor(ms / 1000)) :: string
end

local function needs(t: Thread): boolean
	return t.status == "needs_you"
end

local function thread_row(t: Thread): Node
	local function get(): Thread
		return t
	end
	return kx.Row {
		size = "md",
		selected = function()
			return store.is_open(get().id)
		end,
		on_click = function()
			nav.go("thread")
			store.open(get().id)
		end,
		kx.AgentMark {
			agent = get().harness_id,
			live = function()
				return get().status == "running"
			end,
			needs_you = function()
				return needs(get())
			end,
		},
		ui.text {
			function()
				return get().title
			end,
			literal = true,
			truncate = true,
			grow = 1,
			min_w = 0,
			text_size = "$font.size.md",
			weight = function()
				return if get().unread then "$font.weight.semibold" else "$font.weight.regular"
			end,
			color = function()
				return if get().status == "done" and not get().unread then "$text.secondary" else "$text.primary"
			end,
		},
		ui.text {
			function()
				local t = get()
				return if needs(t) then "needs you" else ago(t.updated)
			end,
			literal = true,
			shrink = 0,
			text_size = "$font.size.xs",
			tabular = true,
			color = function()
				return if needs(get()) then "$kx.attention" else "$text.tertiary"
			end,
		},
	}
end

-- The threads, grouped under their projects in the order they last moved.
type Group = { id: string, name: string, threads: { Thread } }
local groups = ui.computed(function(): { Group }
	local out: { Group } = {}
	local by: { [string]: Group } = {}
	for _, t in store.threads:get() do
		local g = by[t.project_id]
		if not g then
			g = { id = t.project_id, name = t.project, threads = {} }
			by[t.project_id] = g
			table.insert(out, g)
		end
		table.insert(g.threads, t)
	end
	return out
end)

local function group_view(g: Group): Node
	local threads = g.threads
	return ui.column {
		w_full = true,
		gap = "$kx.space.hair",
		ui.text {
			g.name,
			literal = true,
			px = "$space.2",
			pt = "$space.3",
			pb = "$space.1",
			text_size = "$font.size.xs",
			weight = "$font.weight.medium",
			color = "$text.tertiary",
			truncate = true,
		},
		ui.each(threads, thread_row, {
			key = function(t: Thread)
				return t.id
			end,
			w_full = true,
			gap = "$kx.space.hair",
			animate = { duration = 260, ease = kx.motion.EASE },
			enter = { opacity = { 0, 1 }, duration = 200 },
			exit = { opacity = { 1, 0 }, duration = 140 },
		}),
	}
end

local waiting = ui.computed(function(): number
	local n = 0
	for _, t in store.threads:get() do
		if needs(t) then
			n += 1
		end
	end
	return n
end)

local function way_in(icon: string, label: string, keys: string, on_click: () -> (), trailing: Node?): Node
	return kx.Row {
		size = "sm",
		label = label,
		on_click = on_click,
		selected = function()
			return nav.route:get() == string.lower(label)
		end,
		ui.column { color = "$text.secondary", kx.Icon { name = icon, size = 16 } },
		ui.text { label, literal = true, grow = 1, text_size = "$font.size.md" },
		trailing or kx.Hint { keys, literal = true },
	}
end

function M.view(): Node
	local empty = ui.computed(function()
		return #store.threads:get() == 0
	end)
	return ui.column {
		w = "$kx.sidebar",
		h_full = true,
		shrink = 0,
		bg = function()
			return if kx.material.see_through() then "$surface.sunken" else "$kx.solid"
		end,
		border_sides = { 0, 1, 0, 0 },
		border_color = "$border.subtle",
		-- The brand, where the title bar is.
		ui.row {
			h = "$kx.titlebar",
			w_full = true,
			px = "$space.4",
			gap = "$space.2",
			items = "center",
			window_drag = true,
			ui.column { color = "$accent", kx.Icon { name = "orbit", size = 18, label = "Keplorayx" } },
			ui.text {
				"Keplorayx",
				literal = true,
				font = "$kx.display",
				weight = "$font.weight.semibold",
				text_size = "$font.size.md",
				letter_spacing = -0.1,
			},
		},
		ui.column {
			w_full = true,
			px = "$space.2",
			gap = "$kx.space.hair",
			way_in("pen-new-square", "New thread", "Ctrl N", function()
				nav.go("home")
				store.home()
			end),
			way_in("inbox", "Inbox", "", function()
				nav.go("inbox")
			end, ui.show(
				function()
					return waiting:get() > 0
				end,
				ui.row {
					h = 18,
					min_w = 18,
					px = "$kx.space.tight",
					rounded = "$radius.full",
					bg = "$kx.attention.soft",
					items = "center",
					justify = "center",
					ui.text {
						function()
							return tostring(waiting:get())
						end,
						literal = true,
						tabular = true,
						text_size = "$font.size.xs",
						weight = "$font.weight.semibold",
						color = "$kx.attention",
					},
				}
			)),
			way_in("magnifer", "Search", "Ctrl K", function()
				nav.focus_ray()
			end),
		},
		-- The threads.
		ui.scroll {
			grow = 1,
			w_full = true,
			min_h = 0,
			mt = "$space.2",
			px = "$space.2",
			ui.show(
				empty,
				kx.Empty {
					icon = "constellation",
					title = "No threads yet",
					hint = "Ask anything from Home. Each agent shows up here in its own light.",
				},
				ui.each(groups, group_view, {
					key = function(g: Group)
						return g.id
					end,
					w_full = true,
					gap = "$space.1",
					pb = "$space.4",
				})
			),
		},
		-- The foot: settings, and where your data lives.
		ui.column {
			w_full = true,
			px = "$space.2",
			py = "$space.2",
			gap = "$kx.space.hair",
			border_sides = { 1, 0, 0, 0 },
			border_color = "$border.subtle",
			kx.Row {
				size = "sm",
				label = "Settings",
				selected = function()
					return nav.route:get() == "settings"
				end,
				on_click = function()
					nav.go("settings")
				end,
				ui.column { color = "$text.secondary", kx.Icon { name = "settings", size = 16 } },
				ui.text { "Settings", literal = true, grow = 1 },
				kx.Hint { "Ctrl ,", literal = true },
			},
			ui.row {
				px = "$space.2",
				h = 26,
				gap = "$space.2",
				items = "center",
				ui.column { color = "$text.tertiary", kx.Icon { name = "local-only", size = 14 } },
				kx.Hint { "Local only · on this device", literal = true, truncate = true },
			},
		},
	}
end

return M
