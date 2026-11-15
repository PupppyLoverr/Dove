--!strict
-- The Buddies space's sidebar, as Grok Bot's: the pinned buddies in a grid
-- of three (a face and a name each), the rest as rows (a face, a name and
-- what it's doing), then a way to the hidden ones; at its foot, the way back
-- to the chats and **Connect apps** (Settings → Connections), with how many
-- are connected. A secondary click on any buddy opens its menu.
--
--   Roster { collapsed = state, width = state, top = number? }
--   roster.active()   whether the window is in the Buddies space now

local ds = require("@ds")
local Icon = require("@ds/icon")
local ContextMenu = require("@ds/context_menu")
local route = require("@app/screens/shell/route")
local store = require("@app/kx/buddies/store")
local Avatar = require("@app/kx/buddies/avatar")

local M = {}

-- A row's height and face, a tile's face, the roster's inset and the grid's gap.
local ROW, FACE, TILE_FACE, PAD, GAP = 40, 26, 46, 8, 4

local DOING: { [string]: string } = {
	working = "Working",
	connecting = "Starting",
	plan = "Planning",
	input = "Waiting on you",
	approval = "Needs your approval",
	failed = "Stopped on an error",
	limited = "At its limit",
}

local function doing(b: store.Buddy): string
	return DOING[store.standing(b) or ""] or b.role or ""
end

function M.active(): boolean
	local cur: any = route.current:get()
	if cur.name == "plugin" and cur.plugin == "kx" and (cur.page == "buddies" or cur.page == "buddy") then
		return true
	end
	return cur.name == "session" and store.is_chat(cur.session)
end

-- The buddy whose chat is open is the one selected (and read).
ui.effect(function()
	local cur: any = route.current:get()
	if cur.name == "session" then
		local b = store.by_chat(cur.session)
		if b and store.selected:peek() ~= b.id then
			store.selected:set(b.id)
		end
		if b and b.unread then
			store.set_unread(b.id, false)
		end
	end
end)

-- A dot while it waits on you or is unread; a star while a routine is on.
local function badge(b: store.Buddy): () -> string?
	return function(): string?
		local s = store.standing(b)
		if b.unread or s == "input" or s == "approval" then
			return "dot"
		end
		for _, a in store.automations:get() or {} do
			if a.buddy == b.id and a.enabled ~= false then
				return "star"
			end
		end
		return nil
	end
end

local function menu(b: store.Buddy, index: number): { any }
	local pinned = store.is_pinned(b, index)
	return {
		{ label = if pinned then "Unpin" else "Pin to the top", icon = "pin", on_select = function()
			store.toggle_pin(b.id, index)
		end },
		{ label = if b.unread then "Mark as read" else "Mark as unread", icon = "bell", on_select = function()
			store.set_unread(b.id, not b.unread)
		end },
		{ separator = true },
		{ label = "Settings", icon = "settings", on_select = function()
			store.show(b, "settings")
		end },
		{ label = "Save as a template…", icon = "download-minimalistic", on_select = function()
			store.export(b, function(path: string?, why: string?)
				if path then
					ds.toast.show { title = "Saved as a template", body = path, tone = "success" }
				elseif why then
					ds.toast.show { title = "It couldn't be saved", body = why, tone = "danger" }
				end
			end)
		end },
		{ label = "Copy conversation ID", icon = "copy", disabled = if b.chat then nil else "It has no chat yet", on_select = function()
			if b.chat then
				ui.clipboard.write(b.chat)
			end
		end },
		{ separator = true },
		{ label = "Hide from sidebar", icon = "eye-closed", on_select = function()
			store.set_hidden(b.id, true)
		end },
	}
end

local function chosen(b: store.Buddy): () -> boolean
	return function(): boolean
		return store.selected:get() == b.id
	end
end

local function standing(b: store.Buddy): () -> string?
	return function(): string?
		return store.standing(b)
	end
end

local function held(b: store.Buddy, index: number, child: Node, ring: string): Node
	return ContextMenu {
		label = `{b.name}'s actions`,
		ring = ring,
		shrink = 0,
		items = function(): any
			return menu(b, index)
		end,
		child,
	}
end

-- A pinned buddy: a tile, its face over its name.
local function tile(b: store.Buddy, index: number, w: () -> number): Node
	local on = chosen(b)
	return held(
		b,
		index,
		ui.button {
			label = b.name,
			w = w,
			h = 78,
			pt = "$space.2",
			gap = "$space.1",
			direction = "column",
			items = "center",
			rounded = "$radius.lg",
			bg = function(): string
				return if on() then "$zeron.selected.flat.bg" else "#00000000"
			end,
			hover = { bg = "$hover" },
			on_click = function()
				store.open(b)
			end,
			Avatar { look = b.look, seed = b.id, size = TILE_FACE, live = true, standing = standing(b), badge = badge(b) },
			ui.text { b.name, literal = true, truncate = true, max_w = w, px = "$space.1", color = "$text.primary", text_size = "$font.size.xs", weight = if b.unread then "$font.weight.semibold" else "$font.weight.medium" },
		},
		"$radius.lg"
	)
end

-- The rest: a row each.
local function row(b: store.Buddy, index: number): Node
	local on = chosen(b)
	return held(
		b,
		index,
		ui.button {
			label = b.name,
			w_full = true,
			h = ROW,
			shrink = 0,
			px = "$space.2",
			gap = "$space.2",
			items = "center",
			rounded = "$radius.lg",
			bg = function(): string
				return if on() then "$zeron.selected.flat.bg" else "#00000000"
			end,
			hover = { bg = "$hover" },
			on_click = function()
				store.open(b)
			end,
			Avatar { look = b.look, seed = b.id, size = FACE, live = true, standing = standing(b), badge = badge(b) },
			ui.column {
				grow = 1,
				min_w = 0,
				gap = 0,
				items = "start",
				ui.text { b.name, literal = true, truncate = true, w_full = true, color = "$text.primary", text_size = "$font.size.md", weight = if b.unread then "$font.weight.semibold" else "$font.weight.medium" },
				ui.text {
					function(): string
						return doing(b)
					end,
					literal = true,
					truncate = true,
					w_full = true,
					color = function(): string
						local st = store.standing(b)
						return if st == "input" or st == "approval" then "$warning" else "$text.tertiary"
					end,
					text_size = "$font.size.xs",
					hidden = function(): boolean
						return doing(b) == ""
					end,
				},
			},
		},
		"$radius.lg"
	)
end

-- A new buddy: its name and its face. It opens in its chat, its drawer on
-- its settings, to be given a job.
local CreateDialog = ui.component(function(): Node
	local name = ui.use_state("")
	local look = ui.use_state({ shape = "star4", color = "amber", face = "open" } :: Avatar.Look)
	return ds.Dialog {
		open = store.create_open,
		title = "Create a buddy",
		size = "md",
		confirm = {
			label = "Create",
			on_click = function()
				local words = string.gsub(name:peek(), "^%s*(.-)%s*$", "%1")
				store.save({
					name = if words == "" then "New buddy" else words,
					role = "",
					instructions = "",
					permissions = "draft",
					models = {},
					look = look:peek(),
					pinned = true,
				}):after(function(id: any)
					name:set("")
					if type(id) == "string" then
						-- (Once the list has it.)
						ui.after(100, function()
							local b = store.by_id(id)
							if b then
								store.show(b, "settings")
							end
						end)
					end
				end, function(why: any)
					ds.toast.show { title = "The buddy couldn't be made", body = tostring(why), tone = "danger" }
				end)
			end,
		},
		ui.column {
			w_full = true,
			gap = "$space.4",
			items = "center",
			Avatar { look = function(): Avatar.Look?
				return look:get()
			end, seed = "new", size = 88 },
			ds.TextField { value = name, placeholder = "Name your buddy", w_full = true, autofocus = true },
			Avatar.Picker { look = look, seed = "new" },
		},
	}
end)

local connections = ui.shared("kx.connections", {} :: any)

local function connected_count(): number
	local apps = connections:get().apps
	local n = 0
	if type(apps) == "table" then
		for _ in apps do
			n += 1
		end
	end
	return n
end

M.Roster = ui.component(function(props: { collapsed: State<boolean>, width: State<number>, top: number? }): Node
	local query = ui.use_state("")
	local searching = ui.use_state(false)
	local show_hidden = ui.use_state(false)
	local function placed(): { { b: store.Buddy, index: number, pinned: boolean } }
		local q = string.lower(query:get())
		local out = {}
		for i, b in store.list:get() or {} do
			local hit = q == "" or string.find(string.lower(b.name or ""), q, 1, true) ~= nil or string.find(string.lower(b.role or ""), q, 1, true) ~= nil
			if hit and (not b.hidden or show_hidden:get()) then
				table.insert(out, { b = b, index = i, pinned = store.is_pinned(b, i) and not b.hidden })
			end
		end
		return out
	end
	local pinned = ui.computed(function(): { any }
		local out = {}
		for _, p in placed() do
			if p.pinned then
				table.insert(out, p)
			end
		end
		return out
	end)
	local rest = ui.computed(function(): { any }
		local out = {}
		for _, p in placed() do
			if not p.pinned then
				table.insert(out, p)
			end
		end
		return out
	end)
	local hidden_count = ui.computed(function(): number
		local n = 0
		for _, b in store.list:get() or {} do
			if b.hidden then
				n += 1
			end
		end
		return n
	end)
	local function tile_w(): number
		-- (A point short of a third: the grid never wraps to two.)
		return math.max(56, math.floor((props.width:get() - PAD * 2 - GAP * 2) / 3) - 8)
	end
	local function key(p: any): string
		local b = p.b
		local l = b.look or {}
		return `{b.id}:{p.index}:{b.name}:{b.role or ""}:{l.shape or ""}{l.color or ""}{l.face or ""}{l.extra or ""}{l.image or ""}:{b.chat or ""}:{tostring(b.unread)}:{tostring(b.hidden)}`
	end
	return ui.column {
		id = "buddies-roster",
		on_pointer_move = Avatar.track,
		on_hover = function(inside: boolean)
			if not inside then
				Avatar.untrack()
			end
		end,
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
		CreateDialog {},
		ui.row {
			w_full = true,
			h = 32,
			shrink = 0,
			px = "$space.2",
			gap = "$space.1",
			items = "center",
			ui.text { "Buddies", literal = true, grow = 1, color = "$text.secondary", text_size = "$font.size.sm", weight = "$font.weight.medium" },
			ds.IconButton {
				icon = "search" :: any,
				label = "Search buddies",
				size = "sm",
				tone = "quiet",
				on_click = function()
					searching:set(not searching:peek())
					if not searching:peek() then
						query:set("")
					end
				end,
			},
			ds.IconButton {
				icon = "plus" :: any,
				label = "New buddy",
				size = "sm",
				tone = "quiet",
				on_click = function()
					store.create_open:set(true)
				end,
			},
		},
		ui.show(function(): boolean
			return searching:get()
		end, ui.div {
			w_full = true,
			shrink = 0,
			ds.TextField { value = query, placeholder = "Search buddies…", w_full = true, autofocus = true },
		}),
		ui.scroll {
			w_full = true,
			grow = 1,
			min_h = 0,
			ui.column {
				w_full = true,
				gap = "$space.3",
				ui.show(function(): boolean
					return #(store.list:get() or {}) == 0
				end, ui.text { "No buddies yet", literal = true, px = "$space.2", py = "$space.1", color = "$text.tertiary", text_size = "$font.size.sm" }),
				ui.each(pinned, function(p: any): Node
					return tile(p.b, p.index, tile_w)
				end, {
					w_full = true,
					direction = "row",
					flex_wrap = true,
					gap = ui.raw(GAP, "the grid's gap"),
					key = key,
				}),
				ui.each(rest, function(p: any): Node
					return row(p.b, p.index)
				end, {
					w_full = true,
					gap = "$zeron.list.gap",
					key = key,
				}),
				ui.show(function(): boolean
					return hidden_count:get() > 0
				end, ui.button {
					label = "Hidden buddies",
					h = 26,
					px = "$space.2",
					items = "center",
					rounded = "$radius.md",
					bg = "#00000000",
					hover = { bg = "$hover" },
					on_click = function()
						if show_hidden:peek() then
							store.show_all_hidden()
							show_hidden:set(false)
						else
							show_hidden:set(true)
						end
					end,
					ui.text {
						function(): string
							return if show_hidden:get() then "Unhide them all" else `Show hidden ({hidden_count:get()})`
						end,
						literal = true,
						color = "$text.tertiary",
						text_size = "$font.size.xs",
					},
				}),
			},
		},
		-- Its foot: back to the chats, and Connect apps.
		ui.row {
			w_full = true,
			shrink = 0,
			gap = "$space.1",
			items = "center",
			ds.IconButton {
				icon = "alt-arrow-left" :: any,
				label = "Back to chats",
				size = "sm",
				tone = "quiet",
				on_click = function()
					route.new_chat()
				end,
			},
			ui.button {
				label = "Connect apps",
				h = 30,
				px = "$space.3",
				gap = "$space.2",
				items = "center",
				rounded = "$radius.full",
				bg = "$hover",
				hover = { bg = "$pressed" },
				on_click = function()
					route.settings("connections")
				end,
				Icon { name = "widget-add" :: any, size = 14, color = "$text.secondary" },
				ui.text { "Connect apps", literal = true, color = "$text.primary", text_size = "$font.size.sm" },
				ui.text {
					function(): string
						local n = connected_count()
						return if n > 0 then tostring(n) else ""
					end,
					literal = true,
					color = "$text.tertiary",
					text_size = "$font.size.xs",
				},
			},
			ui.div { grow = 1 },
			ds.IconButton {
				icon = "widget" :: any,
				label = "Templates and import",
				size = "sm",
				tone = "quiet",
				on_click = function()
					route.plugin_page("kx", "buddies")
				end,
			},
		},
	}
end)

return M
