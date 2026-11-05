--!strict
-- The notch: Keplorayx's island, hanging from the top edge of the screen
-- over every window, as Dove's Dynamic Island was (its notch/: hidden, slim,
-- alert and expanded). Its window is only as big as what shows (src/notch.rs
-- `notch.size`), and lets clicks through unless it's something to click.
--
-- - **Hidden:** nothing at all; while an agent waits on you, a thin warm line
--   at the edge where the island comes from.
-- - **Slim:** rest the pointer at the top edge over it (`notch.near`) and a
--   capsule drops from the edge: who waits on you, else your agents at work,
--   else the time. It goes 450 ms after the pointer leaves.
-- - **Alert:** without reaching for it, a banner drops when an agent asks to
--   do something (Allow and Deny right there) or a thread finishes or fails
--   (for a few seconds). It never takes the keys.
-- - **Expanded:** click the capsule: Home (the time, a box to ask, your
--   agents, what's playing), Agents (each thread, open, allow, deny), Music
--   (its keys), Notes (kept) and a Timer.

ui.theme.strict("warn")
local kx = require("@kx")
kx.setup()

local store = require("@kx/store")

type Waiting = { thread_id: string, title: string, harness: string, harness_id: string, request_id: string, tool: string, summary: string }
type Now = { hour: number, minute: number, weekday: string, day: number, month: string }

local fixture = store.fixture ~= ""
local attention: State<{ Waiting }> = if fixture
	then ui.state("fx.attention", ((store :: any).fixture_data.attention or {}) :: { Waiting })
	else ui.shared("kx.attention", {} :: { Waiting })
local near: State<boolean> = if fixture then ui.state("fx.near", true) else ui.shared("notch.near", false)
local now: State<Now> = if fixture
	then ui.state("fx.now", { hour = 21, minute = 42, weekday = "Thursday", day = 5, month = "November" } :: Now)
	else ui.shared("kx.now", { hour = 0, minute = 0, weekday = "", day = 0, month = "" } :: Now)

local first = ui.computed(function(): Waiting?
	return attention:get()[1]
end)
local working = ui.computed(function(): { store.Thread }
	local out = {}
	for _, t in store.threads:get() do
		if t.status == "running" then
			table.insert(out, t)
		end
	end
	return out
end)

local glow = kx.motion.breath(0.55, 1)

local function clock(): string
	local t = now:get()
	return string.format("%d:%02d", t.hour, t.minute)
end
local function day(): string
	local t = now:get()
	return if t.weekday == "" then "" else `{string.sub(t.weekday, 1, 3)} {t.day} {string.sub(t.month, 1, 3)}`
end

local function decide(w: Waiting?, decision: string)
	if w and not fixture then
		ui.invoke("kx.decide_for", { thread_id = w.thread_id, request_id = w.request_id, decision = decision })
	end
end

local function open_thread(id: string?)
	if not fixture then
		ui.invoke("notch.focus", id)
	end
end

-- ── What it says ───────────────────────────────────────────────────────

-- A thread finishing or failing, said for a few seconds (as Dove's flash).
type Flash = { id: string, title: string, harness_id: string, failed: boolean }
local flash = ui.state("notch.flash", nil :: Flash?)
local seen: { [string]: string } = {}
local primed = false
ui.effect(function()
	for _, t in store.threads:get() do
		local was = seen[t.id]
		seen[t.id] = t.status
		if primed and was == "running" and (t.status == "done" or t.status == "failed") then
			local f: Flash = { id = t.id, title = t.title, harness_id = t.harness_id, failed = t.status == "failed" }
			flash:set(f)
			ui.after(5000, function()
				if flash:peek() == f then
					flash:set(nil)
				end
			end)
		end
	end
	primed = true
end)

-- Expanded, and which tab.
local expanded = ui.state("notch.expanded", fixture and (store :: any).fixture_data.expanded == true)
local tab = ui.state("notch.tab", "home")
-- (Away, it folds back.)
ui.effect(function()
	if not near:get() and not fixture then
		expanded:set(false)
	end
end)

-- What shows: one state at a time.
local state = ui.computed(function(): string
	if near:get() then
		return if expanded:get() then "expanded" else "slim"
	end
	if first:get() then
		return "alert"
	end
	if flash:get() then
		return "flash"
	end
	return "hidden"
end)

-- Each state's window: its size in points, and whether it takes clicks.
local SIZES: { [string]: { w: number, h: number, interactive: boolean } } = {
	hidden = { w = 120, h = 6, interactive = false },
	slim = { w = 480, h = 64, interactive = true },
	alert = { w = 520, h = 84, interactive = true },
	flash = { w = 420, h = 64, interactive = true },
	expanded = { w = 620, h = 420, interactive = true },
}
ui.effect(function()
	local s = SIZES[state:get()]
	pcall(ui.invoke, "notch.size", s)
end)

-- ── Pieces ─────────────────────────────────────────────────────────────

local function quiet(text: any, size: string?): Node
	return ui.text { text, literal = true, text_size = size or "$font.size.sm", color = "$text.tertiary", truncate = true }
end

local function star(t: store.Thread): Node
	return kx.AgentMark { agent = t.harness_id, size = 8, live = function()
		return true
	end }
end

local function stars(): Node
	return ui.row {
		gap = 4,
		items = "center",
		ui.each(working, star, {
			key = function(t: store.Thread)
				return t.id
			end,
			gap = 4,
			items = "center",
		}),
	}
end

local function rule(): Node
	return ui.div { w = 1, h = 14, bg = "$border.subtle" }
end

local function keys(...: string): Node
	local row: { [any]: any } = { gap = 3, items = "center" }
	for _, k in { ... } do
		table.insert(row, kx.Kbd { k })
	end
	return ui.row(row)
end

local function asking(w: () -> Waiting?, compact: boolean): Node
	return ui.row {
		gap = "$space.3",
		items = "center",
		w_full = true,
		ui.div { w = 8, h = 8, rounded = "$radius.full", bg = "$kx.attention", opacity = glow, shrink = 0 },
		ui.column {
			gap = 1,
			grow = 1,
			min_w = 0,
			ui.text {
				function()
					local x = w()
					return if x then `{x.harness} wants to run {x.tool}` else ""
				end,
				literal = true,
				truncate = true,
				text_size = "$font.size.sm",
				weight = "$font.weight.medium",
			},
			quiet(function()
				local x = w()
				if not x then
					return ""
				end
				return if compact or x.summary == "" then x.title else x.summary
			end, "$font.size.xs"),
		},
		kx.Button { tone = "ghost", size = "xs", shape = "pill", on_click = function()
			decide(w(), "deny")
		end, "Deny" },
		kx.Button { tone = "attention", size = "xs", shape = "pill", on_click = function()
			decide(w(), "allow_once")
		end, "Allow" },
	}
end

-- ── Slim ───────────────────────────────────────────────────────────────

local slim_resting = ui.row {
	gap = "$space.3",
	items = "center",
	ui.column { color = "$accent", kx.Icon { name = "orbit", size = 15 } },
	ui.text { clock, literal = true, weight = "$font.weight.semibold", tabular = true },
	quiet(day),
	rule(),
	quiet("Ask anything"),
	keys("Alt", "Space"),
}

local slim_working = ui.row {
	gap = "$space.3",
	items = "center",
	stars(),
	ui.text {
		function()
			local n = #working:get()
			return if n == 1 then "1 agent working" else `{n} agents working`
		end,
		literal = true,
		weight = "$font.weight.medium",
		text_size = "$font.size.sm",
	},
	rule(),
	ui.text {
		function()
			local t = working:get()[1]
			return if t then t.title else ""
		end,
		literal = true,
		truncate = true,
		max_w = 200,
		text_size = "$font.size.sm",
		color = "$text.secondary",
	},
}

local slim_kind = ui.computed(function(): string
	if first:get() then
		return "waiting"
	end
	return if #working:get() > 0 then "working" else "resting"
end)

local function slim(): Node
	return ui.presence(slim_kind, function(kind: string): Node
		if kind == "waiting" then
			return ui.column {
				w = 420,
				asking(function()
					return first:get()
				end, true),
			}
		end
		return if kind == "working" then slim_working else slim_resting
	end, { enter = { opacity = { 0, 1 }, duration = 160 }, exit = { opacity = { 1, 0 }, duration = 90 } })
end

-- ── Flash ──────────────────────────────────────────────────────────────

local flash_failed = ui.computed(function(): string
	local f = flash:get()
	return if f and f.failed then "failed" else "done"
end)

local function flash_view(): Node
	return ui.row {
		gap = "$space.3",
		items = "center",
		ui.presence(flash_failed, function(kind: string): Node
			return kx.StatusIcon { kind = if kind == "failed" then "failed" else "done", size = 15 }
		end),
		ui.column {
			gap = 1,
			min_w = 0,
			max_w = 300,
			ui.text {
				function()
					local f = flash:get()
					return if f and f.failed then "Stopped with an error" else "Finished"
				end,
				literal = true,
				text_size = "$font.size.sm",
				weight = "$font.weight.medium",
			},
			quiet(function()
				local f = flash:get()
				return if f then f.title else ""
			end, "$font.size.xs"),
		},
	}
end

-- ── Expanded ───────────────────────────────────────────────────────────

local ask_text = ui.state("notch.ask", "")
local ask_busy = ui.state("notch.ask.busy", false)
local function ask()
	local q = ask_text:peek()
	if string.match(q, "^%s*$") or ask_busy:peek() or fixture then
		return
	end
	ask_busy:set(true)
	store.send(q, { access = "supervised" }):after(function()
		ask_text:set("")
		ask_busy:set(false)
		local t = store.thread:peek()
		open_thread(if t then t.id else nil)
	end, function(why: any)
		ask_busy:set(false)
		kx.toast { title = "Couldn't ask", body = tostring(if type(why) == "table" then why.message else why), kind = "error" }
	end)
end

local playing = ui.state("notch.playing", "")
local function read_playing()
	if fixture then
		return
	end
	local ok, v = pcall(ui.call_sync, "widget.playing")
	if ok and type(v) == "string" and v ~= playing:peek() then
		playing:set(v)
	end
end
ui.effect(function()
	if state:get() == "expanded" then
		read_playing()
	end
end)

local function media(icon: string, what: string, name: string): Node
	return kx.Button {
		tone = "ghost",
		size = "sm",
		icon = true,
		label = name,
		on_click = function()
			pcall(ui.invoke, "widget.media", what)
			ui.after(600, read_playing)
		end,
		kx.Icon { name = icon, size = 16 },
	}
end

local function song(part: number): () -> string
	return function()
		local p = playing:get()
		if p == "" then
			return if part == 1 then "Nothing playing" else "Spotify, VLC, foobar2000…"
		end
		local artist, title = string.match(p, "^(.-) %- (.+)$")
		return if part == 1 then (title or p) else (artist or "")
	end
end

local function music_row(big: boolean): Node
	return ui.row {
		gap = "$space.3",
		items = "center",
		w_full = true,
		ui.column {
			w = if big then 56 else 36,
			h = if big then 56 else 36,
			rounded = "$radius.lg",
			bg = "$kx.accent.soft",
			items = "center",
			justify = "center",
			color = "$accent",
			shrink = 0,
			kx.Icon { name = "music-note", size = if big then 22 else 16 },
		},
		ui.column {
			grow = 1,
			min_w = 0,
			ui.text { song(1), literal = true, truncate = true, weight = "$font.weight.medium", text_size = "$font.size.sm" },
			quiet(song(2), "$font.size.xs"),
		},
		media("media-previous", "previous", "Previous"),
		media("media-play", "play", "Play or pause"),
		media("media-next", "next", "Next"),
	}
end

local function thread_row(t: store.Thread): Node
	local waits: Waiting? = nil
	for _, w in attention:peek() do
		if w.thread_id == t.id then
			waits = w
		end
	end
	return ui.button {
		w_full = true,
		gap = "$space.2",
		items = "center",
		rounded = "$radius.md",
		px = "$space.2",
		py = 5,
		bg = "#00000000",
		hover = { bg = "$kx.accent.soft" },
		on_click = function()
			open_thread(t.id)
		end,
		kx.AgentMark { agent = t.harness_id, size = 14 },
		ui.text { t.title, literal = true, truncate = true, grow = 1, min_w = 0, text_size = "$font.size.sm" },
		if waits
			then ui.row {
				gap = "$space.1",
				kx.Button { tone = "ghost", size = "xs", shape = "pill", on_click = function()
					decide(waits, "deny")
				end, "Deny" },
				kx.Button { tone = "attention", size = "xs", shape = "pill", on_click = function()
					decide(waits, "allow_once")
				end, "Allow" },
			}
			elseif t.status == "running" then kx.Spinner { size = 12 }
			elseif t.status == "failed" then kx.StatusIcon { kind = "failed", size = 12 }
			else nil,
	}
end

local function threads(n: number): State<{ store.Thread }>
	return ui.computed(function(): { store.Thread }
		local out = {}
		for _, t in store.threads:get() do
			if t.status == "running" or t.status == "needs_you" then
				table.insert(out, t)
			end
		end
		for _, t in store.threads:get() do
			if #out >= n then
				break
			end
			if t.status ~= "running" and t.status ~= "needs_you" then
				table.insert(out, t)
			end
		end
		return out
	end)
end
local home_threads = threads(3)
local all_threads = threads(9)

local function thread_list(list: State<{ store.Thread }>): Node
	return ui.each(list, thread_row, {
		key = function(t: store.Thread)
			return `{t.id}:{t.status}`
		end,
		gap = 2,
	})
end

local function heading(text: string): Node
	return ui.text { text, literal = true, text_size = "$font.size.xs", weight = "$font.weight.medium", color = "$text.tertiary" }
end

local function home(): Node
	return ui.column {
		gap = "$space.4",
		w_full = true,
		ui.row {
			gap = "$space.4",
			items = "end",
			w_full = true,
			ui.text { clock, literal = true, font = "$kx.display", text_size = ui.raw(44, "the island's clock"), tabular = true },
			ui.column {
				pb = 6,
				quiet(function()
					return string.upper(now:get().weekday)
				end, "$font.size.xs"),
				ui.text {
					function()
						local t = now:get()
						return `{t.day} {t.month}`
					end,
					literal = true,
					weight = "$font.weight.medium",
				},
			},
		},
		ui.row {
			gap = "$space.2",
			items = "center",
			w_full = true,
			kx.Field {
				value = ask_text,
				placeholder = "Ask an agent…",
				label = "Ask an agent",
				grow = 1,
				on_commit = ask,
			},
			kx.Button { tone = "primary", size = "sm", busy = ask_busy, on_click = ask, "Ask" },
		},
		ui.column { gap = "$space.1", w_full = true, heading("AGENTS"), thread_list(home_threads) },
		music_row(false),
	}
end

local note = if fixture then ui.state("notch.note.fx", "Ship the island before the demo.") else ui.persist("notch.note", "")

-- The timer: its end, and what's left (ticking only while it runs).
local timer_end = ui.persist("notch.timer.end", 0)
local timer_left = ui.state("notch.timer.left", 0)
local function tick()
	local left = math.max(0, timer_end:peek() - os.time())
	timer_left:set(left)
	if left > 0 then
		ui.after(1000, tick)
	elseif timer_end:peek() > 0 then
		timer_end:set(0)
		flash:set({ id = "timer", title = "Your timer is done", harness_id = "", failed = false })
		ui.after(5000, function()
			flash:set(nil)
		end)
	end
end
if timer_end:peek() > os.time() then
	tick()
end
local function start_timer(minutes: number)
	timer_end:set(os.time() + minutes * 60)
	tick()
end

local function timer(): Node
	local presets: { [any]: any } = { gap = "$space.2", items = "center" }
	for _, m in { 5, 15, 25, 50 } do
		table.insert(presets, kx.Button { tone = "secondary", size = "sm", on_click = function()
			start_timer(m)
		end, `{m} min` })
	end
	return ui.column {
		gap = "$space.4",
		w_full = true,
		items = "center",
		pt = "$space.4",
		ui.text {
			function()
				local l = timer_left:get()
				return string.format("%d:%02d", math.floor(l / 60), l % 60)
			end,
			literal = true,
			font = "$kx.display",
			text_size = ui.raw(64, "the timer's figures"),
			tabular = true,
		},
		ui.row(presets),
		ui.show(function()
			return timer_left:get() > 0
		end, kx.Button { tone = "ghost", size = "sm", on_click = function()
			timer_end:set(0)
			timer_left:set(0)
		end, "Stop" }),
	}
end

local TABS = {
	{ id = "home", label = "Home", icon = "home" },
	{ id = "agents", label = "Agents", icon = "bot" },
	{ id = "music", label = "Music", icon = "music-note" },
	{ id = "notes", label = "Notes", icon = "note-text" },
	{ id = "timer", label = "Timer", icon = "timer" },
}

local function tabs(): Node
	local row: { [any]: any } = { gap = "$space.1", items = "center" }
	for _, t in TABS do
		table.insert(
			row,
			ui.button {
				h = 28,
				px = "$space.3",
				gap = "$space.1",
				items = "center",
				rounded = "$radius.full",
				bg = function()
					return if tab:get() == t.id then "$kx.accent.soft" else "#00000000"
				end,
				hover = { bg = "$kx.accent.soft" },
				color = function()
					return if tab:get() == t.id then "$text.primary" else "$text.tertiary"
				end,
				label = t.label,
				on_click = function()
					tab:set(t.id)
				end,
				kx.Icon { name = t.icon, size = 13 },
				ui.text { t.label, literal = true, text_size = "$font.size.xs", weight = "$font.weight.medium" },
			}
		)
	end
	return ui.row(row)
end

local function page(id: string): Node
	if id == "agents" then
		return ui.scroll { w_full = true, h_full = true, ui.column { w_full = true, thread_list(all_threads) } }
	elseif id == "music" then
		return ui.column { w_full = true, pt = "$space.6", music_row(true) }
	elseif id == "notes" then
		return ui.textarea {
			value = note,
			placeholder = "A note to keep…",
			w_full = true,
			grow = 1,
			text_size = "$font.size.sm",
			bg = "#00000000",
			border = 0,
		}
	elseif id == "timer" then
		return timer()
	end
	return home()
end

local function expanded_view(): Node
	return ui.column {
		w_full = true,
		h_full = true,
		gap = "$space.4",
		ui.row {
			w_full = true,
			justify = "between",
			items = "center",
			tabs(),
			kx.Button {
				tone = "ghost",
				size = "xs",
				icon = true,
				label = "Open Keplorayx",
				on_click = function()
					open_thread(nil)
				end,
				kx.Icon { name = "arrow-up-right", size = 14 },
			},
		},
		ui.column {
			w_full = true,
			grow = 1,
			min_h = 0,
			ui.presence(tab, page, { w_full = true, h_full = true, enter = { opacity = { 0, 1 }, duration = 140 }, exit = { opacity = 0, duration = 60 }, mode = "wait" }),
		},
	}
end

-- ── The island ─────────────────────────────────────────────────────────

-- Its glass: night, whatever the app's theme; the top tucked above the
-- window's edge so only its lower curve shows, as a notch's.
local function island(content: Node, props: { [string]: any }?): Node
	local p: { [any]: any } = {
		mt = ui.raw(-28, "the top corners tucked above the screen's edge"),
		pt = ui.raw(28, "the content below the tucked edge"),
		px = "$space.5",
		pb = "$space.2",
		rounded = 24,
		bg = "$kx.sky.deep",
		border = 1,
		border_color = function()
			return if first:get() then "$kx.attention.soft" else "$border.subtle"
		end,
		inset_shadow = function()
			return if first:get() then "$kx.lip.inset_strong" else "$kx.lip.inset"
		end,
		shadow = "$shadow.lg",
		items = "center",
		justify = "center",
		content,
	}
	for k, v in props or {} do
		p[k] = v
	end
	return ui.column(p)
end

local function view(kind: string): Node
	if kind == "hidden" then
		return ui.show(first, ui.div { w = 96, h = 3, rounded = 3, bg = "$kx.attention", opacity = glow })
	elseif kind == "slim" then
		return ui.button {
			on_click = function()
				expanded:set(true)
			end,
			label = "Keplorayx: open the island",
			island(slim(), { h = 64 + 28 - 8 }),
		}
	elseif kind == "alert" then
		return ui.column { w = 500, island(asking(function()
			return first:get()
		end, false), { w_full = true, h = 84 + 28 - 8 }) }
	elseif kind == "flash" then
		return ui.button {
			on_click = function()
				local f = flash:peek()
				open_thread(if f and f.id ~= "timer" then f.id else nil)
			end,
			label = "Open the thread",
			island(flash_view(), { h = 64 + 28 - 8 }),
		}
	end
	return ui.column {
		w = 600,
		h = 404,
		island(expanded_view(), { w_full = true, h_full = true, items = "start", justify = "start", pt = ui.raw(28 + 12, "below the tucked edge, and room") }),
	}
end

return ui.row {
	theme = "orbit-night",
	w_full = true,
	h_full = true,
	justify = "center",
	items = "start",
	ui.presence(state, view, {
		enter = { y = { -40, 0 }, opacity = { 0, 1 }, scale = { 0.96, 1 }, duration = 260, ease = "cubicBezier(0.34, 1.4, 0.64, 1)" },
		exit = { y = { 0, -30 }, opacity = { 1, 0 }, duration = 140, ease = kx.motion.EXIT },
		mode = "wait",
	}),
}
