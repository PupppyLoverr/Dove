--!strict
-- A compact Grok-Bot-style header strip shown at the top of a buddy's chat,
-- keeping its avatar, role, live standing, and one-click jumps to Profile,
-- Computer/Desktop, Automations, and Activity right beside the conversation.

local ds = require("@ds")
local Icon = require("@ds/icon")
local route = require("@app/screens/shell/route")
local store = require("@app/kx/buddies/store")
local Avatar = require("@app/kx/buddies/avatar")
local motion = require("@ds/motion")

local M = {}

local STANDING: { [string]: string } = {
	working = "Working",
	connecting = "Starting",
	plan = "Planning",
	input = "Waiting on you",
	approval = "Needs your approval",
	failed = "Failed",
	limited = "At its limit",
	done = "Done",
	waiting = "Waiting",
}

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
	return "$text.tertiary"
end

-- To one part of its settings, in its drawer.
local function jump(buddy_id: string, tab_id: string)
	store.selected:set(buddy_id)
	store.page_tab:set(tab_id)
	store.inspector_open:set(true)
	store.inspector_tab:set("settings")
end

-- A hairline under the bar that runs while it works, in its colour.
local Strip = ui.component(function(props: { buddy: store.Buddy }): Node
	local t = ui.use_state(0)
	local stop = ui.every(33, function()
		local s = store.standing(props.buddy)
		if (s == "working" or s == "connecting" or s == "plan") and not motion.reduced_now() then
			t:set(t:peek() + 1)
		elseif t:peek() ~= 0 then
			t:set(0)
		end
	end)
	ui.on_cleanup(stop)
	local fill = Avatar.resolve_color(props.buddy.look, props.buddy.id).fill
	return ui.canvas {
		w_full = true,
		h = 2,
		shrink = 0,
		draw = function(c, w: number, h: number)
			local n = t:get()
			if n == 0 then
				return
			end
			local seg = w * 0.24
			local x = (n * 7) % (w + seg) - seg
			local l = math.max(0, x)
			local r = math.min(w, x + seg)
			if r > l then
				c:rect(l, 0, r - l, h, fill)
			end
		end,
	}
end)

function M.Bar(session_id: () -> string?): Node
	local buddy_id = ui.computed(function(): string
		local b = store.by_chat(session_id())
		return if b then b.id else ""
	end)
	return ui.presence(function(): string
		return buddy_id:get()
	end, function(id: string): Node
		if id == "" then
			return ui.div {}
		end
		local b = store.by_id(id)
		if not b then
			return ui.div {}
		end
		local buddy: store.Buddy = b
		local has_computer = buddy.computer and (buddy.computer.kind == "local" or buddy.computer.kind == "boat" or buddy.computer.kind == "shared")
		return ui.column {
			w_full = true,
			shrink = 0,
			ui.row {
				w_full = true,
				shrink = 0,
				h = 48,
				px = "$space.4",
				gap = "$space.2",
				items = "center",
				ui.div { grow = 1, basis = 0 },
				-- Its pill, in the middle: its face, its name, and a dot while
				-- it's doing something. Pressed: its drawer.
				ui.button {
					label = `{buddy.name}: show or hide its drawer`,
					h = 34,
					pl = "$space.1",
					pr = "$space.3",
					gap = "$space.2",
					items = "center",
					rounded = "$radius.full",
					bg = "$zeron.secondary.bg",
					hover = { bg = "$zeron.secondary.hover" },
					on_click = function()
						store.inspector_open:set(not store.inspector_open:peek())
					end,
					Avatar {
						look = buddy.look,
						seed = buddy.id,
						size = 26,
						live = true,
						standing = function(): string?
							return store.standing(buddy)
						end,
					},
					ui.text { buddy.name, literal = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.semibold" },
					ui.div {
						w = 6,
						h = 6,
						rounded = "$radius.full",
						bg = function(): string
							return standing_color(buddy)
						end,
						hidden = function(): boolean
							return STANDING[store.standing(buddy) or ""] == nil
						end,
					},
				},
				ui.row {
					grow = 1,
					basis = 0,
					justify = "end",
					gap = "$space.1",
					items = "center",
					if has_computer
						then ds.IconButton {
							icon = "monitor" :: any,
							label = "Show its desktop",
							size = "sm",
							tone = "quiet",
							on_click = function()
								store.open_desktop(buddy, function(ok: boolean, why: string?)
									if not ok then
										ds.toast.show { title = "Its desktop couldn't be shown", body = why, tone = "danger" }
									end
								end)
							end,
						}
						else nil,
					ds.Button {
						tone = "ghost",
						size = "sm",
						on_click = function()
							store.export(buddy, function(path: string?, why: string?)
								if path then
									ds.toast.show { title = "Saved as a template", body = path, tone = "success" }
								elseif why then
									ds.toast.show { title = "It couldn't be saved", body = why, tone = "danger" }
								end
							end)
						end,
						ui.text { "Share", literal = true },
					},
					ds.IconButton {
						icon = "info-circle" :: any,
						label = "Show or hide its drawer",
						size = "sm",
						tone = "quiet",
						lit = function(): boolean
							return store.inspector_open:get()
						end,
						on_click = function()
							store.inspector_open:set(not store.inspector_open:peek())
						end,
					},
				},
			},
			Strip { buddy = buddy },
		}
	end, { w_full = true, shrink = 0 })
end

-- ── The drawer: the buddy beside its chat ────────────────────────────────
--
--   Drawer(session_id)   a column at the chat's right, while its chat shows
--                        and store.inspector_open holds: its face (pressed,
--                        its shape and colour to choose), name and role, and
--                        two tabs: Activity (a routine to make, its computer,
--                        its agent, its routines) and Library (what's in its
--                        folder, newest first, each opened in the panel).

-- Its width: wider while it shows its settings, which have forms.
local function width(): number
	return if store.inspector_tab:get() == "settings" then 420 else 304
end

local function ago(t: number?): string
	if not t or t <= 0 then
		return ""
	end
	local d = os.time() - t
	if d < 60 then
		return "Just now"
	elseif d < 3600 then
		return `{d // 60}m ago`
	elseif d < 86400 then
		return `{d // 3600}h ago`
	elseif d < 86400 * 7 then
		return `{d // 86400}d ago`
	end
	return os.date("%d %b", t) :: any
end

local function upcoming(t: number?): string
	if not t then
		return "Off"
	end
	local d = t - os.time()
	if d <= 60 then
		return "Due now"
	elseif d < 3600 then
		return `In {d // 60}m`
	elseif d < 86400 then
		return `In {d // 3600}h`
	end
	return os.date("%a %H:%M", t) :: any
end

local function card(props: { [any]: any }): Node
	local box: { [any]: any } = {
		w_full = true,
		p = "$space.3",
		gap = "$space.2",
		rounded = "$radius.lg",
		bg = "$zeron.secondary.bg",
	}
	for k, v in props do
		box[k] = v
	end
	return ui.column(box)
end

-- One fact about it, as a quiet row: what it is, and its value; a press
-- goes where it's changed.
local function fact(lead: Node, label: string, value: () -> string, on_click: () -> ()): Node
	return ui.button {
		label = label,
		w_full = true,
		h = 36,
		px = "$space.2",
		gap = "$space.3",
		items = "center",
		rounded = "$radius.md",
		bg = "#00000000",
		hover = { bg = "$hover" },
		on_click = on_click,
		ui.div { w = 16, shrink = 0, items = "center", justify = "center", lead },
		ui.text { label, literal = true, w = 68, shrink = 0, color = "$text.tertiary", text_size = "$font.size.sm" },
		ui.text { value, literal = true, grow = 1, truncate = true, color = "$text.primary", text_size = "$font.size.sm" },
	}
end

local function activity(buddy: store.Buddy): Node
	local kind = buddy.computer and buddy.computer.kind or "none"
	local function routines(): { any }
		local out = {}
		for _, a in store.automations:get() or {} do
			if a.buddy == buddy.id then
				table.insert(out, a)
			end
		end
		return out
	end
	local function awake(): boolean
		if kind == "local" then
			local c = store.computer_of(buddy)
			return c ~= nil and (c.status == "running" or c.status == "ready")
		elseif kind == "boat" then
			local b = store.boat_of(buddy)
			return b ~= nil and (b.state == "running" or b.state == "ready")
		end
		return false
	end
	if kind == "local" and not store.computers_read:peek() then
		store.read_computers()
	end
	local primary = store.primary_agent(buddy)
	return ui.column {
		w_full = true,
		gap = "$zeron.space.hair",
		fact(Icon { name = "monitor", size = 15, color = "$text.tertiary" }, "Desktop", function(): string
			if kind == "none" then
				return "None"
			end
			return `{if kind == "boat" then "Cloud" else "This PC"}, {if awake() then "awake" else "asleep"}`
		end, function()
			if kind == "none" then
				jump(buddy.id, "computer")
			else
				store.open_desktop(buddy, function(ok: boolean, why: string?)
					if not ok then
						ds.toast.show { title = "Its desktop couldn't be shown", body = why, tone = "danger" }
					end
				end)
			end
		end),
		fact(
			if primary and primary.mark
				then ds.AgentMark { label = primary.label, seed = primary.harness, mark = primary.mark, size = 14, bare = true }
				else Icon { name = "tuning", size = 15, color = "$text.tertiary" },
			"Runs on",
			function(): string
				return if primary then `{primary.label}{if primary.model then " · " .. primary.model else ""}` else "Nothing yet"
			end,
			function()
				jump(buddy.id, "profile")
			end
		),
		fact(Icon { name = "clock-circle", size = 15, color = "$text.tertiary" }, "Routines", function(): string
			local n = #routines()
			return if n == 0 then "None" else tostring(n)
		end, function()
			jump(buddy.id, "automations")
		end),
		ui.each(ui.computed(function(): { any }
			return routines()
		end), function(a: any): Node
			return ui.row {
				w_full = true,
				h = 28,
				pl = ui.raw(38, "under the facts' values"),
				pr = "$space.2",
				gap = "$space.2",
				items = "center",
				ui.text { a.title or a.prompt, literal = true, grow = 1, truncate = true, color = if a.enabled == false then "$text.tertiary" else "$text.secondary", text_size = "$font.size.sm" },
				ui.text { if a.enabled == false then "Off" else upcoming(a.next), literal = true, color = "$text.tertiary", text_size = "$font.size.xs" },
			}
		end, {
			w_full = true,
			key = function(a: any): string
				return `{a.id}:{a.title or ""}:{tostring(a.enabled)}:{a.next or 0}`
			end,
		}),
	}
end

local Library = ui.component(function(props: { buddy: store.Buddy }): Node
	local buddy = props.buddy
	local files = ui.use_state(nil :: { any }?)
	ui.call("kx.buddy.files", buddy.id):after(function(list: any)
		files:set(if type(list) == "table" then list else {})
	end, function()
		files:set({})
	end)
	local function open(f: any)
		store.ensure_chat(buddy, function(chat: string?)
			if not chat then
				return
			end
			local ok, panel = pcall(function(): any
				return (require :: any)("@app/screens/panel")
			end)
			if ok and panel and panel.open_file then
				panel.open_file(chat, f.path)
			else
				pcall(ui.invoke, "kx.artifacts.open", f.full)
			end
		end)
	end
	return ui.column {
		w_full = true,
		gap = "$space.2",
		ui.show(function(): boolean
			return files:get() == nil
		end, ui.column { w_full = true, ds.SkeletonRow { lead = 28, title = 120, line = 80 }, ds.SkeletonRow { lead = 28, title = 150, line = 70 }, ds.SkeletonRow { lead = 28, title = 100, line = 90 } }),
		ui.show(function(): boolean
			local f = files:get()
			return f ~= nil and #f == 0
		end, ui.text { "Nothing yet. What it writes in its folder shows here.", literal = true, wrap = true, px = "$space.1", color = "$text.tertiary", text_size = "$font.size.xs" }),
		ui.presence(function(): string
			local f = files:get()
			return if f and f[1] then f[1].path .. ":" .. tostring(f[1].modified) else ""
		end, function(key: string): Node
			local f = files:peek()
			if key == "" or not f then
				return ui.div {}
			end
			local first = f[1]
			return ui.button {
				label = first.title,
				w_full = true,
				p = "$space.3",
				gap = "$space.1",
				direction = "column",
				items = "start",
				rounded = "$radius.lg",
				bg = "$zeron.secondary.bg",
				hover = { bg = "$zeron.secondary.hover" },
				on_click = function()
					open(first)
				end,
				ui.text { "Latest", literal = true, color = "$text.tertiary", text_size = "$font.size.xs" },
				ui.text { first.title, literal = true, w_full = true, wrap = true, color = "$text.primary", text_size = "$font.size.sm", weight = "$font.weight.semibold" },
				ui.row {
					gap = "$space.1",
					items = "center",
					Avatar { look = buddy.look, seed = buddy.id, size = 14 },
					ui.text { `{buddy.name} · {ago(first.modified)}`, literal = true, color = "$text.tertiary", text_size = "$font.size.xs" },
				},
			}
		end, { w_full = true }),
		ui.each(ui.computed(function(): { any }
			return files:get() or {}
		end), function(f: any): Node
			return ui.button {
				label = f.title,
				w_full = true,
				h = 36,
				px = "$space.2",
				gap = "$space.2",
				items = "center",
				rounded = "$radius.md",
				bg = "#00000000",
				hover = { bg = "$hover" },
				on_click = function()
					open(f)
				end,
				Icon { name = "document", size = 14, color = "$text.secondary" },
				ui.text { f.title, literal = true, grow = 1, truncate = true, color = "$text.primary", text_size = "$font.size.sm" },
				ui.text { ago(f.modified), literal = true, color = "$text.tertiary", text_size = "$font.size.xs" },
			}
		end, {
			w_full = true,
			gap = 0,
			key = function(f: any): string
				return `{f.path}:{f.modified}`
			end,
		}),
	}
end)

local Inside = ui.component(function(props: { id: string }): Node
	local picking = ui.use_state(false)
	local b = store.by_id(props.id)
	if not b then
		return ui.div {}
	end
	local buddy: store.Buddy = b
	local look = ui.use_state((buddy.look or {}) :: Avatar.Look)
	local function tab_button(id: string, words: string): Node
		return ui.button {
			label = words,
			h = 28,
			px = "$space.3",
			items = "center",
			rounded = "$radius.full",
			bg = function(): string
				return if store.inspector_tab:get() == id then "$zeron.secondary.bg" else "#00000000"
			end,
			hover = { bg = "$hover" },
			on_click = function()
				store.inspector_tab:set(id)
			end,
			ui.text { words, literal = true, text_size = "$font.size.sm", color = function(): string
				return if store.inspector_tab:get() == id then "$text.primary" else "$text.secondary"
			end },
		}
	end
	return ui.column {
		w_full = true,
		h_full = true,
		min_h = 0,
		gap = "$space.3",
		ui.row {
			w_full = true,
			shrink = 0,
			items = "center",
			ds.IconButton {
				icon = "close" :: any,
				label = "Close the drawer",
				size = "sm",
				tone = "quiet",
				on_click = function()
					store.inspector_open:set(false)
				end,
			},
			ui.div { grow = 1 },
		},
		ui.column {
			w_full = true,
			shrink = 0,
			items = "center",
			gap = "$space.2",
			ui.button {
				label = "Change its look",
				rounded = "$radius.full",
				bg = "#00000000",
				on_click = function()
					picking:set(not picking:peek())
				end,
				Avatar { look = function(): Avatar.Look?
					return look:get()
				end, seed = buddy.id, size = 72, live = true, standing = function(): string?
					return store.standing(buddy)
				end },
			},
			ui.text { buddy.name, literal = true, color = "$text.primary", text_size = "$font.size.lg", weight = "$font.weight.semibold" },
			if buddy.role and buddy.role ~= "" then ui.text { buddy.role, literal = true, color = "$text.tertiary", text_size = "$font.size.sm" } else nil,
		},
		ui.show(function(): boolean
			return picking:get()
		end, card {
			shrink = 0,
			Avatar.Picker {
				look = look,
				seed = buddy.id,
				on_change = function(l: Avatar.Look)
					store.save({ id = buddy.id, name = buddy.name, look = l }):after(function() end, function() end)
				end,
			},
		}),
		ui.row {
			w_full = true,
			shrink = 0,
			justify = "center",
			gap = "$space.1",
			tab_button("activity", "Activity"),
			tab_button("library", "Library"),
			tab_button("settings", "Settings"),
		},
		ui.scroll {
			w_full = true,
			grow = 1,
			min_h = 0,
			ui.presence(function(): string
				return store.inspector_tab:get()
			end, function(which: string): Node
				if which == "library" then
					return Library { buddy = buddy }
				elseif which == "settings" then
					-- (Its settings, as the buddy's page had them: loaded when
					-- first asked for.)
					return (require :: any)("@app/kx/buddies/page").Settings(buddy)
				end
				return activity(buddy)
			end, { w_full = true }),
		},
	}
end)

local Holder = ui.component(function(props: { session_id: () -> string? }): Node
	local session_id = props.session_id
	local buddy_id = ui.computed(function(): string
		local b = store.by_chat(session_id())
		return if b and store.inspector_open:get() then b.id else ""
	end)
	return ui.presence(function(): string
		return buddy_id:get()
	end, function(id: string): Node
		if id == "" then
			return ui.div {}
		end
		return ui.column {
			w = width,
			on_pointer_move = Avatar.track, on_hover = function(inside: boolean) if not inside then Avatar.untrack() end end,
			h_full = true,
			shrink = 0,
			px = "$space.3",
			py = "$space.3",
			Inside { id = id },
		}
	end, { h_full = true, shrink = 0 })
end)

function M.Drawer(session_id: () -> string?): Node
	return Holder { session_id = session_id }
end

-- The docked composer keeps clear of the drawer: it centres on the chat
-- beside it. (Read, never written: what the dock asks is worked out from
-- the chat showing, so nothing loops as pages come and go.)
do
	local ok, Dock = pcall(function(): any
		return (require :: any)("@app/screens/chat/dock")
	end)
	if ok and Dock then
		Dock.right_of = function(): number
			local cur: any = route.current:get()
			if cur.name == "session" and store.inspector_open:get() and store.by_chat(cur.session) then
				return width()
			end
			return 0
		end
		if Dock.right_ready then
			Dock.right_ready:set(Dock.right_ready:peek() + 1)
		end
	end
end

return M
