--!strict
-- The Buddies space's sidebar: a roster, where the chats' sidebar lists
-- sessions. It stands in the same place, at the same width, while a buddies'
-- page or a buddy's chat shows (screens/shell/sidebar swaps the two), and
-- folds away with the same toggle.
--
--   Roster { collapsed = state, width = state, top = number? }
--
-- Each row is a buddy: its face (looking aside while it works, up while it
-- waits on you), its name and role, and a dot for how it stands. A press
-- opens its chat (made on the first press); its "…" opens its page.
--
--   roster.active()   whether the window is in the Buddies space now

local ds = require("@ds")
local Icon = require("@ds/icon")
local route = require("@app/screens/shell/route")
local store = require("@app/kx/buddies/store")
local Avatar = require("@app/kx/buddies/avatar")

local M = {}

-- A row's height, its face, and the roster's inset (the chats' sidebar's).
local ROW, FACE, PAD = 44, 28, 8

function M.active(): boolean
	local cur: any = route.current:get()
	if cur.name == "plugin" and cur.plugin == "kx" and (cur.page == "buddies" or cur.page == "buddy") then
		return true
	end
	return cur.name == "session" and store.is_chat(cur.session)
end

-- The buddy whose chat is open is the one selected: its row, and its page
-- when "…" is pressed.
ui.effect(function()
	local cur: any = route.current:get()
	if cur.name == "session" then
		local b = store.by_chat(cur.session)
		if b and store.selected:peek() ~= b.id then
			store.selected:set(b.id)
		end
	end
end)

local function standing_color(b: store.Buddy): string
	local s = store.standing(b)
	if s == "input" or s == "approval" then
		return "$warning"
	elseif s == "failed" or s == "limited" then
		return "$danger"
	elseif s == "working" or s == "connecting" or s == "plan" then
		return "$accent"
	elseif s == "done" then
		return "$success"
	end
	return "#00000000"
end

local function tab(icon: string, words: string, selected: () -> boolean, on_click: () -> ()): Node
	return ui.button {
		label = words,
		w_full = true,
		h = 30,
		shrink = 0,
		px = "$space.2",
		gap = "$space.2",
		items = "center",
		rounded = "$radius.md",
		bg = function(): string
			return if selected() then "$zeron.selected.flat.bg" else "#00000000"
		end,
		hover = { bg = "$hover" },
		active = { bg = "$pressed" },
		color = "$text.secondary",
		on_click = on_click,
		Icon { name = icon :: any, size = 16 },
		ui.text { words, literal = true, grow = 1, text_size = "$font.size.md", color = function(): string
			return if selected() then "$text.primary" else "$text.secondary"
		end },
	}
end

local function row(b: store.Buddy): Node
	local function chosen(): boolean
		return store.selected:get() == b.id
	end
	return ui.row {
		w_full = true,
		h = ROW,
		shrink = 0,
		items = "center",
		rounded = "$radius.lg",
		bg = function(): string
			return if chosen() then "$zeron.selected.flat.bg" else "#00000000"
		end,
		hover = { bg = "$hover" },
		ui.button {
			label = b.name,
			grow = 1,
			min_w = 0,
			h_full = true,
			pl = "$space.2",
			gap = "$space.2",
			items = "center",
			bg = "#00000000",
			on_click = function()
				store.open(b)
			end,
			Avatar { look = b.look, seed = b.id, size = FACE, standing = function(): string?
				return store.standing(b)
			end },
			ui.column {
				grow = 1,
				min_w = 0,
				gap = 0,
				items = "start",
				ui.text { b.name, literal = true, truncate = true, w_full = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.medium" },
				ui.text {
					if b.role and b.role ~= "" then b.role else "No role yet",
					literal = true,
					truncate = true,
					w_full = true,
					color = "$text.tertiary",
					text_size = "$font.size.xs",
				},
			},
			ui.div { w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = function(): string
				return standing_color(b)
			end },
		},
		ds.IconButton {
			icon = "more-horizontal" :: any,
			label = `{b.name}'s profile and routines`,
			size = "sm",
			tone = "quiet",
			on_click = function()
				store.selected:set(b.id)
				route.plugin_page("kx", "buddy")
			end,
		},
	}
end

M.Roster = ui.component(function(props: { collapsed: State<boolean>, width: State<number>, top: number? }): Node
	local function on_page(id: string): boolean
		local cur: any = route.current:get()
		return cur.name == "plugin" and cur.plugin == "kx" and cur.page == id
	end
	return ui.column {
		id = "buddies-roster",
		w = function(): number
			return if props.collapsed:get() then 0 else props.width:get()
		end,
		hidden = function(): boolean
			return props.collapsed:get()
		end,
		h_full = true,
		shrink = 0,
		overflow = "hidden",
		px = ui.raw(PAD, "the sidebar's inset"),
		pt = ui.raw((props.top or 0) + 4, "the title bar's strip over it, then its own inset"),
		pb = ui.raw(PAD, "the sidebar's inset"),
		gap = "$space.2",
		-- Its head: which space this is, and the way back to the chats.
		ui.row {
			w_full = true,
			h = 30,
			shrink = 0,
			px = "$space.2",
			gap = "$space.2",
			items = "center",
			ui.text { "Buddies", literal = true, grow = 1, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.semibold" },
			ui.text {
				function(): string
					return tostring(#(store.list:get() or {}))
				end,
				literal = true,
				color = "$text.tertiary",
				text_size = "$font.size.xs",
				tabular = true,
			},
		},
		ui.column {
			w_full = true,
			shrink = 0,
			gap = "$zeron.list.gap",
			tab("widget", "The team", function(): boolean
				return on_page("buddies")
			end, function()
				store.selected:set(nil)
				route.plugin_page("kx", "buddies")
			end),
		},
		ui.text { "Your buddies", literal = true, px = "$space.2", pt = "$space.2", shrink = 0, color = "$text.tertiary", text_size = "$font.size.xs", weight = "$font.weight.medium" },
		ui.scroll {
			w_full = true,
			grow = 1,
			min_h = 0,
			ui.column {
				w_full = true,
				gap = "$zeron.list.gap",
				ui.show(function(): boolean
					return #(store.list:get() or {}) == 0
				end, ui.text { "None yet. The team page has four to start from.", literal = true, wrap = true, px = "$space.2", color = "$text.tertiary", text_size = "$font.size.xs" }),
				ui.each(store.list, function(b: any): Node
					return row(b)
				end, {
					key = function(b: any): string
						local l = b.look or {}
						return `{b.id}:{b.name}:{b.role or ""}:{l.shape or ""}{l.color or ""}{l.face or ""}:{b.chat or ""}`
					end,
				}),
			},
		},
		ui.column {
			w_full = true,
			shrink = 0,
			tab("alt-arrow-left", "Back to chats", function(): boolean
				return false
			end, function()
				route.new_chat()
			end),
		},
	}
end)

return M
