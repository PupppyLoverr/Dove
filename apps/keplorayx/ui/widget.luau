--!strict
-- A desktop widget (src/widgets.rs): a small window of glass on the
-- desktop, or floating over it, drawn with the app's design system. Its
-- kind and variant come from `kx.widgets` by this window's `widget.id`:
--
--   clock      digital · analog · world · small
--   calendar   agenda · month · small     (Keplorayx's own calendar, which
--                                          agents add to: `calendar_add`)
--   agents     what's working, and what waits on you
--   threads    the latest threads; a click opens one
--   ask        a field: Enter starts a thread
--   usage      threads this week, by agent
--   system     CPU and memory: bars · small · graph
--   music      what's playing, and its keys
--   note       text you keep
--   custom     an agent's view (a Luau visual, as `show_visual`'s)
--
-- Drag it anywhere (its place is kept); its menu (a secondary click)
-- changes its variant, floats it over your windows, or removes it.

ui.theme.strict("warn")
local kx = require("@kx")
kx.setup()

local store = require("@kx/store")
local Visual = require("@ds/visual")
local ContextMenu = require("@ds/context_menu")

type Widget = { id: string, kind: string, variant: string, title: string?, settings: any, script: string?, on_top: boolean? }
type Event = { id: string, title: string, date: string, time: string?, minutes: number?, notes: string? }
type Now = { hour: number, minute: number, weekday: string, day: number, month: string, month_n: number, year: number, date: string, unix: number }

local fixture = store.fixture ~= ""
local my_id: string = (ui.shared("widget.id", "") :: any):peek()
local widgets: State<{ Widget }> = ui.shared("kx.widgets", {} :: { Widget })
local events: State<{ Event }> = ui.shared("kx.calendar", {} :: { Event })
local now: State<Now> = ui.shared("kx.now", {
	hour = 0,
	minute = 0,
	weekday = "",
	day = 1,
	month = "",
	month_n = 1,
	year = 2026,
	date = "",
	unix = 0,
} :: Now)
local system: State<any> = ui.shared("kx.system", { cpu = 0, ram = 0, ram_used_gb = 0, ram_total_gb = 0 })

local me = ui.computed(function(): Widget
	for _, w in widgets:get() do
		if w.id == my_id then
			return w
		end
	end
	return { id = my_id, kind = "clock", variant = "digital", settings = {} }
end)

local function settings(): any
	local s = me:get().settings
	return if type(s) == "table" then s else {}
end

local function pad(n: number): string
	return string.format("%02d", n)
end

local function label(text: any): Node
	return ui.text { text, literal = true, text_size = "$font.size.xs", color = "$text.tertiary", weight = "$font.weight.medium" }
end

-- ── Clock ───────────────────────────────────────────────────────────────

local function clock_digital(): Node
	return ui.column {
		gap = "$space.1",
		label(function()
			local t = now:get()
			return string.upper(`{t.weekday} {t.day} {t.month}`)
		end),
		ui.text {
			function()
				local t = now:get()
				return `{t.hour}:{pad(t.minute)}`
			end,
			literal = true,
			font = "$kx.display",
			text_size = ui.raw(56, "a clock's figures, the widget's own size"),
			tabular = true,
		},
	}
end

local function clock_analog(): Node
	local function face(c: any, w: number, h: number)
		local t = now:get()
		local cx, cy, r = w / 2, h / 2, math.min(w, h) / 2 - 4
		c:stroke(ui.path():arc(cx, cy, r, 0, 360), "$border.subtle", 1)
		for i = 0, 11 do
			local a = math.rad(i * 30)
			local inner = if i % 3 == 0 then r - 9 else r - 5
			c:stroke(
				ui.path():move(cx + math.sin(a) * inner, cy - math.cos(a) * inner):line(cx + math.sin(a) * (r - 2), cy - math.cos(a) * (r - 2)),
				"$text.tertiary",
				if i % 3 == 0 then 1.5 else 1
			)
		end
		local function hand(angle: number, length: number, width: number, color: string)
			local a = math.rad(angle)
			c:stroke(ui.path():move(cx, cy):line(cx + math.sin(a) * length, cy - math.cos(a) * length), color, width)
		end
		hand((t.hour % 12 + t.minute / 60) * 30, r * 0.5, 3, "$text.primary")
		hand(t.minute * 6, r * 0.78, 2, "$text.primary")
		c:fill(ui.path():arc(cx, cy, 3, 0, 360), "$accent")
	end
	return ui.row {
		gap = "$space.4",
		items = "center",
		h_full = true,
		ui.canvas { w = 116, h = 116, draw = face, label = "Clock" },
		ui.column {
			gap = "$space.1",
			label(function()
				return string.upper(now:get().weekday)
			end),
			ui.text {
				function()
					local t = now:get()
					return `{t.day} {t.month}`
				end,
				literal = true,
				text_size = "$font.size.lg",
				weight = "$font.weight.medium",
			},
		},
	}
end

local DEFAULT_ZONES = {
	{ label = "London", offset = 0 },
	{ label = "New York", offset = -300 },
	{ label = "Tokyo", offset = 540 },
}

local function clock_world(): Node
	local function zones(): { any }
		local z = settings().zones
		return if type(z) == "table" and #z > 0 then z else DEFAULT_ZONES
	end
	local function at(offset: number): string
		local t = now:get()
		local utc = t.unix + (offset or 0) * 60
		local mins = math.floor(utc / 60) % 1440
		return `{math.floor(mins / 60)}:{pad(mins % 60)}`
	end
	local row: { [any]: any } = { gap = "$space.5", w_full = true, justify = "between" }
	for i = 1, 3 do
		table.insert(
			row,
			ui.column {
				gap = "$space.1",
				label(function()
					local z = zones()[i]
					return if z then string.upper(z.label or "") else ""
				end),
				ui.text {
					function()
						local z = zones()[i]
						return if z then at(z.offset) else ""
					end,
					literal = true,
					text_size = "$font.size.xxl",
					tabular = true,
				},
			}
		)
	end
	return ui.row(row)
end

local function clock_small(): Node
	return ui.column {
		gap = "$space.1",
		justify = "center",
		h_full = true,
		ui.text {
			function()
				local t = now:get()
				return `{t.hour}:{pad(t.minute)}`
			end,
			literal = true,
			font = "$kx.display",
			text_size = ui.raw(44, "a clock's figures, the widget's own size"),
			tabular = true,
		},
		label(function()
			local t = now:get()
			return string.upper(`{string.sub(t.weekday, 1, 3)} {t.day}`)
		end),
	}
end

-- ── Calendar ────────────────────────────────────────────────────────────

local function upcoming(n: number): { Event }
	local today = now:get().date
	local out = {}
	for _, e in events:get() do
		if e.date >= today then
			table.insert(out, e)
			if #out >= n then
				break
			end
		end
	end
	return out
end

local MONTHS = { "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec" }
local function when(e: Event): string
	local t = now:get() :: any
	local day = if e.date == t.date
		then "Today"
		elseif e.date == t.tomorrow then "Tomorrow"
		else `{tonumber(string.sub(e.date, 9, 10))} {MONTHS[tonumber(string.sub(e.date, 6, 7)) or 1]}`
	return if e.time then `{day} {e.time}` else day
end

local function event_row(e: Event): Node
	return ui.row {
		gap = "$space.2",
		items = "center",
		w_full = true,
		ui.div { w = 3, h = 16, rounded = "$radius.full", bg = "$accent" },
		ui.text { e.title, literal = true, truncate = true, grow = 1, text_size = "$font.size.sm" },
		ui.text { when(e), literal = true, text_size = "$font.size.xs", color = "$text.tertiary", tabular = true },
	}
end

local function calendar_agenda(n: number): Node
	local list = ui.computed(function(): { Event }
		return upcoming(n)
	end)
	return ui.column {
		gap = "$space.2",
		w_full = true,
		label(function()
			local t = now:get()
			return string.upper(`{t.weekday} {t.day} {t.month}`)
		end),
		ui.show(
			function()
				return #list:get() > 0
			end,
			ui.each(list, event_row, {
				key = function(e: Event)
					return e.id
				end,
				gap = "$space.2",
			}),
			ui.text { "Nothing coming up. Ask an agent to add something.", literal = true, text_size = "$font.size.sm", color = "$text.tertiary", wrap = true }
		),
	}
end

local function calendar_month(): Node
	local function days(): { any }
		local t = now:get()
		local dim = ({ 31, if t.year % 4 == 0 then 29 else 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 })[t.month_n] or 30
		-- Weekday of the first (Zeller's, Monday first).
		local y, m = t.year, t.month_n
		if m < 3 then
			m += 12
			y -= 1
		end
		local h = (1 + math.floor(13 * (m + 1) / 5) + y + math.floor(y / 4) - math.floor(y / 100) + math.floor(y / 400)) % 7
		local first = (h + 5) % 7
		local busy: { [number]: boolean } = {}
		local prefix = `{t.year}-{pad(t.month_n)}-`
		for _, e in events:get() do
			if string.sub(e.date, 1, 8) == prefix then
				busy[tonumber(string.sub(e.date, 9, 10)) or 0] = true
			end
		end
		local out = {}
		for _ = 1, first do
			table.insert(out, { day = 0 })
		end
		for d = 1, dim do
			table.insert(out, { day = d, today = d == t.day, busy = busy[d] })
		end
		return out
	end
	local cells = ui.computed(days)
	local head: { [any]: any } = { w_full = true, justify = "between" }
	for _, d in { "M", "T", "W", "T", "F", "S", "S" } do
		table.insert(head, ui.column { w = 36, items = "center", label(d) })
	end
	return ui.column {
		gap = "$space.2",
		w_full = true,
		ui.text {
			function()
				local t = now:get()
				return `{t.month} {t.year}`
			end,
			literal = true,
			weight = "$font.weight.medium",
		},
		ui.row(head),
		ui.grid {
			columns = 7,
			gap = 2,
			w_full = true,
			ui.each(cells, function(c: any): Node
				return ui.column {
					h = 28,
					items = "center",
					justify = "center",
					rounded = "$radius.full",
					bg = if c.today then "$accent" else nil,
					ui.text {
						if c.day > 0 then tostring(c.day) else "",
						literal = true,
						text_size = "$font.size.xs",
						tabular = true,
						color = if c.today then "$background" else "$text.primary",
					},
					if c.busy and not c.today then ui.div { w = 3, h = 3, rounded = "$radius.full", bg = "$accent" } else nil,
				}
			end),
		},
	}
end

local function calendar_small(): Node
	return ui.column {
		gap = "$space.1",
		justify = "center",
		h_full = true,
		label(function()
			return string.upper(now:get().weekday)
		end),
		ui.text {
			function()
				return tostring(now:get().day)
			end,
			literal = true,
			font = "$kx.display",
			text_size = ui.raw(48, "a date's figure, the widget's own size"),
		},
		ui.text {
			function()
				local next = upcoming(1)[1]
				return if next then next.title else "Nothing today"
			end,
			literal = true,
			truncate = true,
			text_size = "$font.size.xs",
			color = "$text.secondary",
		},
	}
end

-- ── Agents and threads ──────────────────────────────────────────────────

local function thread_row(t: store.Thread): Node
	local waiting = t.status == "needs_you"
	return ui.button {
		w_full = true,
		gap = "$space.2",
		items = "center",
		rounded = "$radius.md",
		px = "$space.1",
		py = 3,
		hover = { bg = "$kx.accent.soft" },
		on_click = function()
			pcall(ui.invoke, "widget.main", t.id)
		end,
		kx.AgentMark { agent = t.harness_id, size = 12 },
		ui.text { t.title, literal = true, truncate = true, grow = 1, text_size = "$font.size.sm" },
		if waiting
			then ui.div { w = 6, h = 6, rounded = "$radius.full", bg = "$kx.attention" }
			elseif t.status == "running" then kx.Spinner { size = 10 }
			else nil,
	}
end

local function thread_list(filter: (store.Thread) -> boolean, n: number, empty: string): Node
	local list = ui.computed(function(): { store.Thread }
		local out = {}
		for _, t in store.threads:get() do
			if filter(t) then
				table.insert(out, t)
				if #out >= n then
					break
				end
			end
		end
		return out
	end)
	return ui.show(
		function()
			return #list:get() > 0
		end,
		ui.each(list, thread_row, {
			key = function(t: store.Thread)
				return `{t.id}:{t.status}`
			end,
			gap = 2,
		}),
		ui.text { empty, literal = true, text_size = "$font.size.sm", color = "$text.tertiary" }
	)
end

local function rows_for(): number
	local v = me:get().variant
	return if v == "small" then 3 elseif v == "large" then 10 else 4
end

local function agents(): Node
	return ui.column {
		gap = "$space.2",
		w_full = true,
		label(function()
			local working, waiting = 0, 0
			for _, t in store.threads:get() do
				if t.status == "running" then
					working += 1
				elseif t.status == "needs_you" then
					waiting += 1
				end
			end
			return string.upper(if waiting > 0 then `{working} working · {waiting} waiting on you` else `{working} working`)
		end),
		thread_list(function(t)
			return t.status == "running" or t.status == "needs_you"
		end, rows_for(), "All quiet."),
	}
end

local function threads(): Node
	return ui.column {
		gap = "$space.2",
		w_full = true,
		label("RECENT THREADS"),
		thread_list(function()
			return true
		end, rows_for(), "No threads yet."),
	}
end

local function usage(): Node
	local by = ui.computed(function(): { any }
		local counts: { [string]: number } = {}
		local week = now:get().unix - 7 * 86400
		for _, t in store.threads:get() do
			if (t.updated or 0) >= week or week <= 0 then
				counts[t.harness] = (counts[t.harness] or 0) + 1
			end
		end
		local out = {}
		for name, n in counts do
			table.insert(out, { name = name, n = n })
		end
		table.sort(out, function(a, b)
			return a.n > b.n
		end)
		return out
	end)
	return ui.column {
		gap = "$space.2",
		w_full = true,
		label("THREADS THIS WEEK"),
		ui.each(by, function(r: any): Node
			local top = math.max(1, (by:peek()[1] or { n = 1 }).n)
			return ui.column {
				gap = 3,
				w_full = true,
				ui.row {
					w_full = true,
					justify = "between",
					ui.text { r.name, literal = true, text_size = "$font.size.sm" },
					ui.text { tostring(r.n), literal = true, text_size = "$font.size.sm", tabular = true, color = "$text.secondary" },
				},
				ui.div {
					h = 4,
					w_full = true,
					rounded = "$radius.full",
					bg = "$kx.accent.soft",
					ui.div { h_full = true, w = ui.raw(math.max(6, 300 * r.n / top), "a bar's share"), max_w = "100%", rounded = "$radius.full", bg = "$accent" },
				},
			}
		end, {
			key = function(r: any)
				return `{r.name}:{r.n}`
			end,
			gap = "$space.2",
		}),
	}
end

-- ── Ask ────────────────────────────────────────────────────────────────

local function ask(): Node
	local text = ui.state("widget.ask", "")
	local busy = ui.state("widget.ask.busy", false)
	local function send()
		local q = text:peek()
		if string.match(q, "^%s*$") or busy:peek() then
			return
		end
		busy:set(true)
		store.send(q, { access = "supervised" }):after(function()
			text:set("")
			busy:set(false)
			local t = store.thread:peek()
			pcall(ui.invoke, "widget.main", if t then t.id else nil)
		end, function(why: any)
			busy:set(false)
			kx.toast { title = "Couldn't ask", body = tostring(if type(why) == "table" then why.message else why), kind = "error" }
		end)
	end
	return ui.column {
		gap = "$space.2",
		w_full = true,
		label("ASK KEPLORAYX"),
		ui.row {
			gap = "$space.2",
			items = "center",
			w_full = true,
			kx.Field { value = text, placeholder = "Ask anything…", label = "Ask Keplorayx", grow = 1, on_submit = send },
			kx.Button { tone = "primary", size = "sm", busy = busy, on_click = send, "Ask" },
		},
	}
end

-- ── System ─────────────────────────────────────────────────────────────

local function meter(name: string, value: () -> number, detail: () -> string): Node
	return ui.column {
		gap = 4,
		w_full = true,
		ui.row {
			w_full = true,
			justify = "between",
			label(name),
			ui.text { detail, literal = true, text_size = "$font.size.xs", tabular = true, color = "$text.secondary" },
		},
		ui.div {
			h = 6,
			w_full = true,
			rounded = "$radius.full",
			bg = "$kx.accent.soft",
			ui.div {
				h_full = true,
				rounded = "$radius.full",
				bg = function()
					return if value() > 85 then "$kx.attention" else "$accent"
				end,
				w = function()
					return ui.raw(math.max(4, value() * 3), "a meter's share")
				end,
				animate = { duration = 400 },
			},
		},
	}
end

local history = ui.state("widget.cpu.history", {} :: { number })
ui.effect(function()
	local cpu = system:get().cpu or 0
	local list = table.clone(history:peek())
	table.insert(list, cpu)
	while #list > 60 do
		table.remove(list, 1)
	end
	history:set(list)
end)

local function system_view(): Node
	local function cpu(): number
		return system:get().cpu or 0
	end
	local function ram(): number
		return system:get().ram or 0
	end
	local v = me:get().variant
	if v == "small" then
		return ui.column {
			gap = "$space.3",
			justify = "center",
			h_full = true,
			ui.column {
				label("CPU"),
				ui.text { function()
					return `{cpu()}%`
				end, literal = true, text_size = "$font.size.xxl", tabular = true },
			},
			ui.column {
				label("MEMORY"),
				ui.text { function()
					return `{ram()}%`
				end, literal = true, text_size = "$font.size.xxl", tabular = true },
			},
		}
	elseif v == "graph" then
		local function graph(c: any, w: number, h: number)
			local list = history:get()
			if #list < 2 then
				return
			end
			local p = ui.path()
			for i, value in list do
				local x = (i - 1) / 59 * w
				local y = h - value / 100 * h
				if i == 1 then
					p:move(x, y)
				else
					p:line(x, y)
				end
			end
			c:stroke(p, "$accent", 1.5)
		end
		return ui.column {
			gap = "$space.2",
			w_full = true,
			ui.row {
				w_full = true,
				justify = "between",
				label("CPU, LAST TWO MINUTES"),
				ui.text { function()
					return `{cpu()}%`
				end, literal = true, text_size = "$font.size.sm", tabular = true },
			},
			ui.canvas { w = 300, h = 64, draw = graph, label = "CPU over time" },
			meter("MEMORY", ram, function()
				local s = system:get()
				return `{s.ram_used_gb} of {s.ram_total_gb} GB`
			end),
		}
	end
	return ui.column {
		gap = "$space.4",
		w_full = true,
		meter("CPU", cpu, function()
			return `{cpu()}%`
		end),
		meter("MEMORY", ram, function()
			local s = system:get()
			return `{s.ram_used_gb} of {s.ram_total_gb} GB`
		end),
	}
end

-- ── Music ──────────────────────────────────────────────────────────────

local playing = ui.state("widget.playing", "")
local function poll_playing()
	local ok, v = pcall(ui.call_sync, "widget.playing")
	if ok and type(v) == "string" and v ~= playing:peek() then
		playing:set(v)
	end
	ui.after(3000, poll_playing)
end

local function music(): Node
	if not fixture and me:peek().kind == "music" then
		poll_playing()
	end
	local function key(icon: string, what: string, name: string): Node
		return kx.Button {
			tone = "ghost",
			size = "sm",
			icon = true,
			label = name,
			on_click = function()
				pcall(ui.invoke, "widget.media", what)
			end,
			kx.Icon { name = icon, size = 16 },
		}
	end
	local function part(i: number): () -> string
		return function()
			local p = playing:get()
			if p == "" then
				return if i == 1 then "Nothing playing" else "Spotify, VLC, foobar2000…"
			end
			local a, b = string.match(p, "^(.-) %- (.+)$")
			return if i == 1 then (b or p) else (a or "")
		end
	end
	return ui.column {
		gap = "$space.2",
		w_full = true,
		label("NOW PLAYING"),
		ui.row {
			gap = "$space.3",
			items = "center",
			w_full = true,
			ui.column {
				w = 44,
				h = 44,
				rounded = "$radius.lg",
				bg = "$kx.accent.soft",
				items = "center",
				justify = "center",
				color = "$accent",
				kx.Icon { name = "music-note", size = 20 },
			},
			ui.column {
				grow = 1,
				min_w = 0,
				ui.text { part(1), literal = true, truncate = true, weight = "$font.weight.medium" },
				ui.text { part(2), literal = true, truncate = true, text_size = "$font.size.sm", color = "$text.secondary" },
			},
		},
		ui.row {
			gap = "$space.1",
			justify = "center",
			w_full = true,
			key("media-previous", "previous", "Previous"),
			key("media-play", "play", "Play or pause"),
			key("media-next", "next", "Next"),
		},
	}
end

-- ── Note ───────────────────────────────────────────────────────────────

local function note(): Node
	local text = ui.state("widget.note", (settings().text or "") :: string)
	local saving: (() -> ())? = nil
	ui.effect(function()
		local t = text:get()
		if saving then
			saving()
		end
		saving = ui.after(800, function()
			local w = me:peek()
			local s = table.clone(settings())
			if s.text ~= t then
				s.text = t
				pcall(ui.invoke, "kx.widget.set", { id = w.id, kind = w.kind, variant = w.variant, settings = s })
			end
		end)
	end)
	return ui.column {
		gap = "$space.2",
		w_full = true,
		h_full = true,
		label("NOTE"),
		ui.textarea { value = text, placeholder = "Write something…", grow = 1, w_full = true, text_size = "$font.size.sm", bg = "#00000000", border = 0 },
	}
end

-- ── Custom: an agent's view ────────────────────────────────────────────

local function custom(): Node
	local w = me:peek()
	if not w.script or w.script == "" then
		return ui.text { "This widget has no view yet.", literal = true, color = "$text.tertiary" }
	end
	return ui.scroll { w_full = true, h_full = true, Visual { id = `widget-{w.id}`, source = w.script, title = w.title or "Widget" } }
end

-- ── The widget ─────────────────────────────────────────────────────────

local function body(): Node
	local w = me:get()
	local k, v = w.kind, w.variant
	if k == "clock" then
		return if v == "analog" then clock_analog() elseif v == "world" then clock_world() elseif v == "small" then clock_small() else clock_digital()
	elseif k == "calendar" then
		return if v == "month" then calendar_month() elseif v == "small" then calendar_small() else calendar_agenda(4)
	elseif k == "agents" then
		return agents()
	elseif k == "threads" then
		return threads()
	elseif k == "ask" then
		return ask()
	elseif k == "usage" then
		return usage()
	elseif k == "system" then
		return system_view()
	elseif k == "music" then
		return music()
	elseif k == "note" then
		return note()
	elseif k == "custom" then
		return custom()
	end
	return ui.text { `No widget called {k}`, literal = true }
end

local VARIANTS: { [string]: { string } } = {
	clock = { "digital", "analog", "world", "small" },
	calendar = { "agenda", "month", "small" },
	system = { "bars", "small", "graph" },
	music = { "medium", "small" },
	ask = { "medium" },
}

local function menu(): { any }
	local w = me:peek()
	local out: { any } = {}
	for _, v in VARIANTS[w.kind] or { "medium", "small", "large" } do
		table.insert(out, {
			label = string.upper(string.sub(v, 1, 1)) .. string.sub(v, 2),
			checked = w.variant == v,
			on_click = function()
				pcall(ui.invoke, "kx.widget.set", { id = w.id, kind = w.kind, variant = v })
			end,
		})
	end
	table.insert(out, { separator = true })
	table.insert(out, {
		label = if w.on_top then "Keep on the desktop" else "Float over windows",
		on_click = function()
			pcall(ui.invoke, "kx.widget.set", { id = w.id, kind = w.kind, variant = w.variant, on_top = not w.on_top })
		end,
	})
	table.insert(out, {
		label = "Remove",
		on_click = function()
			pcall(ui.invoke, "kx.widget.remove", w.id)
		end,
	})
	return out
end

-- Its menu: a secondary click anywhere on it.
local menu_open = ui.state("widget.menu", false)

return ui.column {
	w_full = true,
	h_full = true,
	p = 6,
	ContextMenu {
		items = function()
			return menu()
		end,
		open = menu_open,
		w_full = true,
		h_full = true,
		ring = "$radius.xl",
		ui.column(kx.float_style {
			w_full = true,
			h_full = true,
			rounded = "$radius.xl",
			p = "$space.4",
			gap = "$space.2",
			window_drag = true,
			-- Built again only as its kind or variant changes.
			ui.presence(function(): string
				local w = me:get()
				return `{w.kind}:{w.variant}`
			end, function(_: string): Node
				return body()
			end),
		}),
	},
}
