--!strict
-- The sidebar, on Zeron's shell (ds/sidebar, session_row; MIT, see
-- THIRD_PARTY_NOTICES.md): the way in (New thread, Search, Inbox,
-- Constellation), what you pinned, then every project with its threads
-- hanging from it on a tree's line. Each thread is led by its agent's mark,
-- in its light, and ends in what it's doing: the working ring, a warm
-- glyph when it needs you, a check when it's done, else how long ago. A
-- project folds; past five threads the rest wait behind "Show more". You
-- and Settings sit at the foot, with where your data lives.

local kx = require("@kx")
local Status = require("@kx/status")
local store = require("@kx/store")
local nav = require("@screens/nav")

type Thread = store.Thread

local M = {}

-- A row's height, where its tree's arm meets it, and the trunk's x.
local ROW_H, TRUNK_X, ARM_TIP, BEND = 30, 17, 30, 6
local SHOWN = 5

-- "2m", "3h", "4d", "4 Nov": as short as it can say it.
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

local function kind(t: Thread): Status.Kind
	return kx.status.kind_of(t.status)
end

-- The row's end: the ring or a glyph while there's news, else the time.
local function trailing(get: () -> Thread): Node
	return ui.switch(function(): string
		local k = kind(get())
		return if k == "idle" or (k == "done" and not get().unread) then "time" else "status"
	end, {
		status = kx.StatusIcon {
			kind = function()
				return kind(get())
			end,
			size = 15,
		},
		time = ui.text {
			function()
				return ago(get().updated)
			end,
			literal = true,
			shrink = 0,
			tabular = true,
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		},
	})
end

-- A thread's right-click: zgx's own menu, of Orbit's items.
local function context_menu(get: () -> Thread): { Node }
	local id = get().id
	return {
		kx.MenuItem {
			icon = "pin",
			if nav.pinned:peek()[id] then "Unpin" else "Pin to the top",
			on_click = function()
				nav.toggle_pin(id)
			end,
		},
		kx.MenuItem {
			icon = "arrow-up-right",
			"Open",
			on_click = function()
				nav.go("thread")
				store.open(id)
			end,
		},
		kx.MenuItem {
			icon = "copy",
			"Copy the title",
			on_click = function()
				ui.clipboard.write(get().title)
			end,
		},
	}
end

-- A thread's row. `branch`: on a project's tree (its arm drawn to the
-- mark), and `last` ends the trunk at it.
local function thread_row(get: () -> Thread, branch: boolean, last: (() -> boolean)?): Node
	local body = kx.Row {
		size = "sm",
		pl = if branch then ui.raw(ARM_TIP + 2, "clears the tree's arm") else nil,
		label = function()
			return get().title
		end,
		selected = function()
			return store.is_open(get().id)
		end,
		on_click = function()
			nav.go("thread")
			store.open(get().id)
		end,
		context_menu = function(_e: any)
			return context_menu(get)
		end,
		kx.AgentMark {
			agent = get().harness_id,
			size = 14,
			live = function()
				return get().status == "running"
			end,
			needs_you = function()
				return get().status == "needs_you"
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
				return if get().unread then "$font.weight.medium" else "$font.weight.regular"
			end,
			color = function()
				local t = get()
				return if t.status == "done" and not t.unread then "$text.secondary" else "$text.primary"
			end,
		},
		trailing(get),
	}
	if not branch then
		return body
	end
	return ui.div {
		relative = true,
		w_full = true,
		shrink = 0,
		body,
		ui.canvas {
			absolute = true,
			left = 0,
			top = 0,
			bottom = 0,
			w = ARM_TIP,
			pointer_events = "none",
			draw = function(c: any, _: number, h: number)
				c:fill(
					kx.tree_line.branch {
						x = TRUNK_X,
						mid = ROW_H / 2,
						tip = ARM_TIP - 4,
						h = h,
						bend = BEND,
						on = not (last and last()),
					},
					"$kx.tree"
				)
			end,
		},
	}
end

-- The threads, grouped under their projects in the order they last moved;
-- pinned threads lead, apart.
type Group = { id: string, name: string, threads: { Thread } }
local grouped = ui.computed(function(): { pinned: { Thread }, groups: { Group } }
	local pins = nav.pinned:get()
	local pinned: { Thread } = {}
	local out: { Group } = {}
	local by: { [string]: Group } = {}
	for _, t in store.threads:get() do
		if pins[t.id] then
			table.insert(pinned, t)
			continue
		end
		local g = by[t.project_id]
		if not g then
			g = { id = t.project_id, name = t.project, threads = {} }
			by[t.project_id] = g
			table.insert(out, g)
		end
		table.insert(g.threads, t)
	end
	return { pinned = pinned, groups = out }
end)

local pinned_list = ui.computed(function(): { Thread }
	return grouped:get().pinned
end)
local group_list = ui.computed(function(): { Group }
	return grouped:get().groups
end)

local function by_id(t: Thread): string
	return t.id
end

-- A project's tile: its initial on a lifted square, in the light of the
-- agent it last worked with.
local function tile(g: Group): Node
	local first = g.threads[1]
	return ui.row {
		w = 18,
		h = 18,
		shrink = 0,
		rounded = "$radius.md",
		bg = "$kx.secondary.bg",
		items = "center",
		justify = "center",
		ui.text {
			string.upper(string.sub(g.name, 1, 1)),
			literal = true,
			text_size = "$font.size.xs",
			weight = "$font.weight.semibold",
			color = kx.tokens.agent(if first then first.harness_id else nil),
		},
	}
end

local function group_view(g: Group): Node
	local expanded = ui.state(`sidebar.more.{g.id}`, false)
	local function folded(): boolean
		return nav.folded:get()[g.id] == true
	end
	local shown = ui.computed(function(): { Thread }
		if expanded:get() or #g.threads <= SHOWN then
			return g.threads
		end
		return table.move(g.threads, 1, SHOWN, 1, {})
	end)
	local hidden = #g.threads - SHOWN
	local function row(t: Thread): Node
		return thread_row(function()
			return t
		end, true, function()
			local list = shown:get()
			return list[#list].id == t.id and hidden <= 0
		end)
	end
	return ui.column {
		w_full = true,
		kx.Row {
			size = "sm",
			label = g.name,
			on_click = function()
				local next = table.clone(nav.folded:peek())
				next[g.id] = if next[g.id] then nil else true
				nav.folded:set(next)
			end,
			tile(g),
			ui.text {
				g.name,
				literal = true,
				truncate = true,
				grow = 1,
				min_w = 0,
				text_size = "$font.size.md",
				weight = "$font.weight.medium",
			},
			ui.column {
				color = "$text.tertiary",
				rotate = function(): number
					return if folded() then -90 else 0
				end,
				kx.Icon { name = "chevron-down", size = 12 },
			},
		},
		ui.show(
			function()
				return not folded()
			end,
			ui.column {
				w_full = true,
				ui.each(shown, row, {
					key = by_id,
					w_full = true,
					animate = { duration = 220, ease = kx.motion.EASE },
					enter = { opacity = { 0, 1 }, duration = 180 },
					exit = { opacity = { 1, 0 }, duration = 120 },
				}),
				if hidden > 0
					then kx.Row {
						size = "sm",
						pl = ui.raw(ARM_TIP + 2, "clears the tree's arm"),
						on_click = function()
							expanded:set(not expanded:peek())
						end,
						ui.column {
							color = "$text.tertiary",
							rotate = function(): number
								return if expanded:get() then 180 else 0
							end,
							kx.Icon { name = "chevron-down", size = 12 },
						},
						ui.text {
							function()
								return if expanded:get() then "Show less" else `Show {hidden} more`
							end,
							literal = true,
							text_size = "$font.size.sm",
							color = "$text.tertiary",
						},
					}
					else nil,
			}
		),
	}
end

local waiting = ui.computed(function(): number
	local n = 0
	for _, t in store.threads:get() do
		if t.status == "needs_you" then
			n += 1
		end
	end
	return n
end)

local function way_in(icon: string, label: string, route: string?, keys: string, on_click: () -> (), trailing_node: Node?): Node
	return kx.Row {
		size = "sm",
		label = label,
		on_click = on_click,
		selected = if route
			then function()
				return nav.route:get() == route
			end
			else nil,
		ui.column { color = "$text.secondary", kx.Icon { name = icon, size = 16 } },
		ui.text { label, literal = true, grow = 1, text_size = "$font.size.md" },
		trailing_node or kx.Hint { keys, literal = true },
	}
end

local function section(title: string): Node
	return ui.text {
		title,
		literal = true,
		px = "$space.2",
		pt = "$space.4",
		pb = "$space.1",
		text_size = "$font.size.sm",
		color = "$text.tertiary",
	}
end

local function count_badge(n: () -> number): Node
	return ui.row {
		h = 18,
		min_w = 18,
		px = "$kx.space.tight",
		rounded = "$radius.full",
		bg = "$kx.attention.soft",
		items = "center",
		justify = "center",
		ui.text {
			function()
				return tostring(n())
			end,
			literal = true,
			tabular = true,
			text_size = "$font.size.xs",
			weight = "$font.weight.semibold",
			color = "$kx.attention",
		},
	}
end

local profile_name = ui.persist("kx.profile.name", "")

function M.view(): Node
	local empty = ui.computed(function()
		return #store.threads:get() == 0
	end)
	local width = kx.motion.tween(
		ui.computed(function(): number
			return if nav.sidebar_open:get() then 272 else 0
		end),
		kx.motion.move(272)
	)
	return ui.column {
		w = width,
		h_full = true,
		shrink = 0,
		overflow = "hidden",
		ui.column {
			w = "$kx.sidebar",
			h_full = true,
			-- Where the title bar is: the brand, and the fold.
			ui.row {
				h = "$kx.titlebar",
				w_full = true,
				pl = "$space.4",
				pr = "$space.2",
				gap = "$space.2",
				items = "center",
				window_drag = true,
				ui.column { color = "$text.primary", kx.Icon { name = "orbit", size = 16, label = "Keplorayx" } },
				ui.text {
					"Keplorayx",
					literal = true,
					grow = 1,
					weight = "$font.weight.medium",
					text_size = "$font.size.md",
				},
				kx.Button {
					tone = "ghost",
					size = "sm",
					icon = true,
					label = "Hide the sidebar (Ctrl+B)",
					tooltip = "Hide the sidebar  Ctrl+B",
					on_click = function()
						nav.sidebar_open:set(false)
					end,
					kx.Icon { name = "sidebar", size = 16 },
				},
			},
			ui.column {
				w_full = true,
				px = "$space.2",
				way_in("pen-new-square", "New thread", "home", "Ctrl+N", function()
					nav.go("home")
					store.home()
				end),
				way_in("magnifer", "Search", nil, "Ctrl+K", function()
					nav.focus_ray()
				end),
				way_in(
					"inbox",
					"Inbox",
					"inbox",
					"",
					function()
						nav.go("inbox")
					end,
					ui.show(function()
						return waiting:get() > 0
					end, count_badge(function()
						return waiting:get()
					end))
				),
				way_in("constellation", "Constellation", "constellation", "Ctrl+−", function()
					nav.go("constellation")
				end),
				way_in("users-group", "Team", "team", "", function()
					nav.go("team")
				end),
			},
			ui.scroll {
				grow = 1,
				w_full = true,
				min_h = 0,
				px = "$space.2",
				ui.show(
					empty,
					kx.Empty {
						icon = "constellation",
						title = "No threads yet",
						hint = "Ask anything from Home. Each agent shows up here in its own light.",
					},
					ui.column {
						w_full = true,
						pb = "$space.4",
						ui.show(function()
							return #pinned_list:get() > 0
						end, ui.column {
							w_full = true,
							section("Pinned"),
							ui.each(pinned_list, function(t: Thread)
								return thread_row(function()
									return t
								end, false)
							end, { key = by_id, w_full = true }),
						}),
						section("Projects"),
						ui.each(group_list, group_view, {
							key = function(g: Group)
								return g.id
							end,
							w_full = true,
							gap = "$kx.space.hair",
						}),
					}
				),
			},
			-- The foot: you, Settings, and where your data lives.
			ui.column {
				w_full = true,
				px = "$space.2",
				py = "$space.2",
				gap = "$kx.space.hair",
				kx.Row {
					size = "md",
					label = "Settings",
					selected = function()
						return nav.route:get() == "settings"
					end,
					on_click = function()
						nav.go("settings")
					end,
					ui.row {
						w = 22,
						h = 22,
						shrink = 0,
						rounded = "$radius.full",
						bg = "$accent",
						items = "center",
						justify = "center",
						ui.text {
							function()
								local n = profile_name:get()
								return if n ~= "" then string.upper(string.sub(n, 1, 1)) else "K"
							end,
							literal = true,
							text_size = "$font.size.xs",
							weight = "$font.weight.semibold",
							color = "$accent.text",
						},
					},
					ui.text {
						function()
							local n = profile_name:get()
							return if n ~= "" then n else "You"
						end,
						literal = true,
						grow = 1,
						truncate = true,
						text_size = "$font.size.md",
					},
					ui.column { color = "$text.tertiary", kx.Icon { name = "settings", size = 15 } },
				},
				ui.row {
					px = "$space.2",
					h = 22,
					gap = "$space.2",
					items = "center",
					ui.column { color = "$text.tertiary", kx.Icon { name = "local-only", size = 13 } },
					kx.Hint { "Local only · on this device", literal = true, truncate = true },
				},
			},
		},
	}
end

return M
