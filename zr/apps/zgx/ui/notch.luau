--!strict
-- The notch: Keplorayx's island at the top of the screen, over every
-- window, after Apple's Dynamic Island: a pill hanging from the screen's top
-- edge that grows down out of it and morphs between its shapes on a spring
-- (a little overshoot), black at its top melting into frosted glass below.
-- Its window stays still; src/notch.rs cuts the glass to the pill's shape
-- as it springs (`notch.size { w, h, r }`), and the pill drawn here follows
-- the same spring, so the two move as one. Clicks outside it pass through. Music is what Windows' media controls report
-- (src/media.rs `kx.media`): its cover, title and artist, where it is, and
-- its keys.
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
local kx = require("@app/kx")
kx.setup()

local store = require("@app/kx/store")
-- A thread, read in the island as in the window (its transcript, compact).
local Transcript = require("@ds/transcript").Transcript
-- (The open thread's messages and the reply being written: Zeron's chat.)
local conversation = { messages = store.messages, draft = store.draft }

type Waiting = { thread_id: string, title: string, harness: string, harness_id: string, request_id: string, tool: string, summary: string }
type Now = { hour: number, minute: number, weekday: string, day: number, month: string }

local fixture = store.fixture ~= ""
local attention: State<{ Waiting }> = if fixture
	then ui.state("fx.attention", ((store :: any).fixture_data.attention or {}) :: { Waiting })
	else store.attention :: any
local near: State<boolean> = if fixture then ui.state("fx.near", true) else ui.shared("notch.near", false)
local now: State<Now> = if fixture
	then ui.state("fx.now", { hour = 21, minute = 42, weekday = "Thursday", day = 5, month = "November" } :: Now)
	else ui.shared("kx.now", { hour = 0, minute = 0, weekday = "", day = 0, month = "" } :: Now)

-- What's playing (src/media.rs), and its place now (moved on by the clock
-- while it plays, each second the island shows it).
type Media = { app: string, title: string, artist: string, playing: boolean, position: number, duration: number, at: number, cover: string }
local media: State<Media?> = if fixture
	then ui.state("fx.media", ((store :: any).fixture_data.media) :: Media?)
	else ui.shared("kx.media", nil :: Media?)
local tick = ui.state("notch.tick", 0)
local function position(): number
	tick:get()
	local m = media:get()
	if not m then
		return 0
	end
	local p = if m.playing then m.position + (os.time() - m.at) else m.position
	return if m.duration > 0 then math.min(p, m.duration) else p
end
local function mmss(t: number): string
	t = math.max(0, math.floor(t))
	return string.format("%d:%02d", math.floor(t / 60), t % 60)
end
local function control(what: any)
	if not fixture then
		pcall(ui.invoke, "media.control", what)
	end
end

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

-- Allow or Deny for a thread waiting on you: Zeron answers a request in a
-- chat that's open, so the island opens it (in its own window, the main
-- window's left as it is) and answers as soon as the request arrives.
local wanted = ui.state("notch.wanted", nil :: { thread: string, decision: string }?)
ui.effect(function()
	local want = wanted:get()
	local ask = store.approval:get()
	local open = store.thread:get()
	if want and ask and open and open.id == want.thread then
		store.decide(ask.request_id, want.decision)
		wanted:set(nil)
	end
end)
local function decide(w: Waiting?, decision: string)
	if w and not fixture then
		wanted:set({ thread = w.thread_id, decision = decision })
		store.open(w.thread_id)
	end
end

local function open_thread(id: string?)
	if not fixture then
		ui.invoke("notch.focus", id)
	end
end

-- ── What it says ───────────────────────────────────────────────────────

-- What happened, newest first (the Alerts tab, as Dove's Notifications):
-- each its kind ("done", "failed", "asked"), its thread and when.
type Alert = { kind: string, thread: string, title: string, at: number }
local alerts = if fixture
	then ui.state("notch.alerts.fx", {
		{ kind = "asked", thread = "t2", title = "Approve: run the store migration", at = os.time() - 120 },
		{ kind = "done", thread = "t3", title = "Tidy the settings pages", at = os.time() - 900 },
		{ kind = "failed", thread = "t4", title = "Fix the flaky sign-in redirect", at = os.time() - 3600 },
	} :: { Alert })
	else ui.persist("notch.alerts", {} :: { Alert })
local function remember(kind: string, thread: string, title: string)
	local list = table.clone(alerts:peek())
	table.insert(list, 1, { kind = kind, thread = thread, title = title, at = os.time() })
	while #list > 30 do
		table.remove(list)
	end
	alerts:set(list)
end
-- (An agent asking: once each time it starts waiting.)
local asked_seen: { [string]: boolean } = {}
ui.effect(function()
	local now: { [string]: boolean } = {}
	for _, w in attention:get() do
		local key = w.thread_id .. ":" .. w.request_id
		now[key] = true
		if not asked_seen[key] then
			remember("asked", w.thread_id, `{w.harness} wants to run {w.tool}: {w.title}`)
		end
	end
	asked_seen = now
end)

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
			remember(if f.failed then "failed" else "done", t.id, t.title)
			ui.after(5000, function()
				if flash:peek() == f then
					flash:set(nil)
				end
			end)
		end
	end
	primed = true
end)

-- The timer: its end, and what's left (ticking only while it runs).
local timer_end = ui.persist("notch.timer.end", 0)
local timer_total = ui.persist("notch.timer.total", 0)
local timer_left = ui.state("notch.timer.left", 0)
local function timer_tick()
	local left = math.max(0, timer_end:peek() - os.time())
	timer_left:set(left)
	if left > 0 then
		ui.after(1000, timer_tick)
	elseif timer_end:peek() > 0 then
		timer_end:set(0)
		flash:set({ id = "timer", title = "Your timer is done", harness_id = "", failed = false })
		ui.after(5000, function()
			flash:set(nil)
		end)
	end
end
if timer_end:peek() > os.time() then
	timer_tick()
end
local function start_timer(minutes: number)
	local running = timer_end:peek() > os.time()
	timer_end:set(os.time() + minutes * 60)
	if not running then
		timer_total:set(minutes * 60)
		timer_tick()
	end
end
local function add_minute()
	if timer_end:peek() > os.time() then
		timer_end:set(timer_end:peek() + 60)
		timer_total:set(timer_total:peek() + 60)
		timer_left:set(math.max(0, timer_end:peek() - os.time()))
	end
end

-- (Running, as a live activity: true while it runs, and its time as m:ss.)
local timer_on = ui.computed(function(): boolean
	return timer_left:get() > 0
end)
local function timer_text(): string
	local l = timer_left:get()
	return string.format("%d:%02d", math.floor(l / 60), l % 60)
end

-- Settings › General's "Keep the island out while something happens"
-- (src/kx/notch.rs `set_pin`): on, the island stays at the top while there
-- is a live activity, as the Mac's does; off, it tucks away until you
-- reach for it (and still drops in for what waits on you).
local pin: State<boolean> = if fixture then ui.state("fx.pin", true) else ui.shared("notch.pin", true)

-- What's going on now, each a live activity, the busiest first: agents at
-- work, music playing, a timer running. The compact island shows them all.
local live = ui.computed(function(): { string }
	local out = {}
	if #working:get() > 0 then
		table.insert(out, "agents")
	end
	local m = media:get()
	if m and m.playing then
		table.insert(out, "music")
	end
	if timer_on:get() then
		table.insert(out, "timer")
	end
	return out
end)

-- Expanded, and which tab.
local expanded = ui.state("notch.expanded", fixture and (store :: any).fixture_data.expanded == true)
local tab = ui.state("notch.tab", if fixture and type((store :: any).fixture_data.tab) == "string" then (store :: any).fixture_data.tab else "home")
-- A thread opened in the island (its own, apart from the window's): read
-- it, answer what it asks, write to it.
local chat = ui.state("notch.chat", nil :: string?)
local function open_chat(id: string)
	store.open(id)
	chat:set(id)
	expanded:set(true)
end
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
	-- (Pinned: something's happening, so it stays out, compact.)
	if pin:get() and #live:get() > 0 then
		return "slim"
	end
	return "hidden"
end)

-- Each state's pill, in points: its width and height, its corners, how far
-- below the screen's edge it floats (above it, out of sight, while it
-- rests), and whether it takes clicks. (The Dynamic Island's: compact, a
-- banner, and expanded by what it shows.)
type Shape = { w: number, h: number, r: number, y: number, interactive: boolean }
-- (The gap between the screen's edge and the island.)
local FLOAT = 8
-- (Apple's island: quick, overshooting a touch as it lands, so the shape
-- reads as one thing changing rather than a window swapped. The same as
-- src/kx/notch.rs's `SPRING`, which cuts the window to the pill: change
-- both together, or the cut and the pill drift apart mid-spring.)
local SPRING = { stiffness = 360, damping = 26 }

-- The clock, while music shows and plays.
local function ticking()
	local s = state:peek()
	local m = media:peek()
	if m and m.playing and (s == "expanded" or s == "slim") then
		tick:set(tick:peek() + 1)
		ui.after(1000, ticking)
	end
end
ui.effect(function()
	state:get()
	local m = media:get()
	if m and m.playing then
		ui.after(1000, ticking)
	end
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
		ui.div { w = 8, h = 8, rounded = "$radius.full", bg = "$kx.attention", shrink = 0 },
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
		ui.button {
			h = 28,
			px = 12,
			rounded = "$radius.full",
			items = "center",
			bg = "#ffffff1f",
			hover = { bg = "#ffffff33" },
			active = { scale = 0.95 },
			color = "#ffffffe6",
			label = "Deny",
			on_click = function()
				decide(w(), "deny")
			end,
			ui.text { "Deny", literal = true, text_size = "$font.size.xs", weight = "$font.weight.semibold" },
		},
		ui.button {
			h = 28,
			px = 14,
			rounded = "$radius.full",
			items = "center",
			bg = "$kx.attention",
			hover = { opacity = 0.88 },
			active = { scale = 0.95 },
			color = "#1a1206",
			label = "Allow",
			on_click = function()
				decide(w(), "allow_once")
			end,
			ui.text { "Allow", literal = true, text_size = "$font.size.xs", weight = "$font.weight.bold" },
		},
	}
end

-- ── Music's pieces ───────────────────────────────────────────────────

local function pill(text: string, strong: boolean, run: () -> ()): Node
	return ui.button {
		h = 28,
		px = 12,
		rounded = "$radius.full",
		items = "center",
		bg = if strong then "#ffffff" else "#ffffff1f",
		hover = { bg = if strong then "#ffffffd9" else "#ffffff33" },
		active = { scale = 0.95 },
		color = if strong then "#000000" else "#ffffffe6",
		label = text,
		on_click = run,
		ui.text { text, literal = true, text_size = "$font.size.xs", weight = "$font.weight.semibold" },
	}
end

-- A key on the island. `bare` is a transport glyph: no disc behind it,
-- white, only dimming and shrinking under the pointer.
local function key(icon: string, what: string, name: string, size: number?, bare: boolean?): Node
	local n = size or 16
	local plain = bare == true
	return ui.button {
		w = if plain then n + 28 else n + 18,
		h = if plain then n + 20 else n + 18,
		rounded = "$radius.full",
		items = "center",
		justify = "center",
		bg = "#00000000",
		color = if plain then "#ffffff" else "#ffffffeb",
		hover = if plain then { opacity = 0.7 } else { bg = "#ffffff14" },
		active = { scale = 0.9 },
		label = name,
		on_click = function()
			control(what)
		end,
		kx.Icon { name = icon, size = n },
	}
end

local function cover(size: number, corner: number?): Node
	local round = corner or math.max(6, math.floor(size * 0.2))
	return ui.div {
		w = size,
		h = size,
		shrink = 0,
		rounded = ui.raw(round, "the cover's corner"),
		overflow = "hidden",
		bg = ui.raw("#ffffff16", "a cover before its picture"),
		border = if size >= 36 then 1 else 0,
		border_color = ui.raw("#ffffff33", "the cover's rim"),
		shadow = if size >= 48 then { x = 0, y = 12, blur = 24, spread = -10, color = "#0000008c" } else nil,
		ui.presence(function(): string
			local m = media:get()
			return if m and m.cover ~= "" then m.cover else ""
		end, function(src: string): Node
			if src == "" then
				return ui.column {
					w_full = true,
					h_full = true,
					items = "center",
					justify = "center",
					color = "#ffffffd0",
					kx.Icon { name = "music-note", size = math.floor(size * 0.4) },
				}
			end
			return ui.image { src = src, w_full = true, h_full = true, fit = "cover" }
		end, { enter = { opacity = { 0, 1 }, scale = { 0.92, 1 }, duration = 260 } }),
	}
end

local function song(part: string): () -> string
	return function()
		local m = media:get()
		if not m then
			return if part == "title" then "Nothing playing" else "Play something in Spotify, a browser or Media Player"
		end
		if part == "title" then
			return m.title
		end
		return if m.artist ~= "" then m.artist else m.app
	end
end

-- The waveform, in the cover's light (src/media.rs `tint`): white at its
-- leading edge, the cover's colour by the last bar, a fixed silhouette that
-- only breathes while the track plays.
local wave = ui.state("notch.wave", 0)
local WAVE = { 0.32, 0.58, 0.84, 0.46, 1, 0.64, 0.9, 0.4, 0.7 }
local function tint(): string
	local m = media:get()
	return if m and (m :: any).tint and (m :: any).tint ~= "" then (m :: any).tint else "#e7b4c0"
end
local function channel(hex: string, at: number): number
	return tonumber(string.sub(hex, at, at + 1), 16) or 0
end
local function mix(a: string, b: string, t: number): string
	local function norm(c: string): string
		local h = if string.sub(c, 1, 1) == "#" then string.sub(c, 2) else c
		return if #h >= 6 then string.sub(h, 1, 6) else "ffffff"
	end
	local x, y = norm(a), norm(b)
	local function blend(i: number): number
		local from, to = channel(x, i), channel(y, i)
		return math.clamp(math.floor(from + (to - from) * t + 0.5), 0, 255)
	end
	return string.format("#%02x%02x%02x", blend(1), blend(3), blend(5))
end
local function waveform(bars: number, height: number, width: number?): Node
	local row: { [any]: any } = { gap = ui.raw(2, "space between the bars"), items = "center", h = height, shrink = 0 }
	for i = 1, bars do
		local base = WAVE[(i - 1) % #WAVE + 1]
		table.insert(row, ui.div {
			w = width or 3,
			rounded = "$radius.full",
			h = function()
				local m = media:get()
				local swing = math.abs(math.sin(wave:get() * (0.62 + i * 0.27) + i * 1.35))
				local amount = if m and m.playing then 0.48 + 0.52 * swing else 0.36
				return math.max(3, height * base * amount)
			end,
			bg = function()
				local along = if bars <= 1 then 0 else (i - 1) / (bars - 1)
				return ui.raw(mix("#ffffff", tint(), along * 0.92), "a bar, from white to the cover's light")
			end,
		})
	end
	return ui.row(row)
end

-- ── Slim ───────────────────────────────────────────────────────────────
-- The compact island: something at its leading end, something at its
-- trailing end, a word between when there's room.

local function compact(lead: Node, middle: Node?, trail: Node): Node
	return ui.row {
		w_full = true,
		h_full = true,
		px = 12,
		gap = "$space.2",
		items = "center",
		lead,
		ui.row { grow = 1, min_w = 0, justify = "center", items = "center", middle },
		trail,
	}
end

local function white(text: any, size: string?, weight: string?): Node
	return ui.text { text, literal = true, truncate = true, color = "#ffffffeb", text_size = size or "$font.size.sm", weight = weight or "$font.weight.medium" }
end

-- (Resting, as the Mac's: the time at one end, the mark at the other,
-- nothing between.)
local _ = keys
local slim_resting = compact(
	white(clock, nil, "$font.weight.semibold"),
	nil,
	ui.column { color = "#ffffff8c", kx.Icon { name = "orbit", size = 14 } }
)

local slim_working = compact(
	stars(),
	white(function()
		local t = working:get()[1]
		local n = #working:get()
		if n > 1 then
			return `{n} agents working`
		end
		return if t then t.title else ""
	end, "$font.size.xs"),
	kx.Spinner { size = 13 }
)

-- What the compact island holds: what waits on you, else every live
-- activity at once ("agents+music+timer"), else the time at rest.
local slim_kind = ui.computed(function(): string
	if first:get() then
		return "waiting"
	end
	local l = live:get()
	return if #l == 0 then "resting" else table.concat(l, "+")
end)
local function slim_count(kind: string): number
	if kind == "waiting" or kind == "resting" then
		return 0
	end
	local _, plus = string.gsub(kind, "%+", "")
	return plus + 1
end

-- The timer's ring, small: it empties as the time runs out.
local function timer_ring(size: number, stroke: number): Node
	return ui.canvas {
		w = size,
		h = size,
		shrink = 0,
		draw = function(c, w, h)
			local cx, cy, r = w / 2, h / 2, math.min(w, h) / 2 - stroke / 2 - 0.5
			c:stroke(ui.path():arc(cx, cy, r, 0, 360), "#ffffff26", stroke)
			local total = math.max(1, timer_total:get())
			local f = math.clamp(timer_left:get() / total, 0, 1)
			if f > 0 then
				c:stroke(ui.path():arc(cx, cy, r, -90, 360 * f), "#ffb454", stroke, { cap = "round" })
			end
		end,
	}
end
local function amber(text: any, size: string?): Node
	return ui.text { text, literal = true, color = "#ffc773", text_size = size or "$font.size.xs", weight = "$font.weight.semibold", tabular = true }
end
local slim_timer = compact(timer_ring(18, 2.5), white("Timer", "$font.size.xs"), amber(timer_text, "$font.size.sm"))

-- Several at once: the busiest leads (its mark and its words), and each
-- other sits at the trailing end in a small chip of its own (agents a
-- spinner and how many, music its cover and waveform, a timer its time).
local function agents_title(): string
	local t = working:get()[1]
	local n = #working:get()
	if n > 1 then
		return `{n} agents working`
	end
	return if t then t.title else ""
end
local function lead_of(kind: string): Node
	if kind == "agents" then
		return ui.row { grow = 1, min_w = 0, gap = 8, items = "center", stars(), white(agents_title, "$font.size.xs") }
	elseif kind == "music" then
		return ui.row { grow = 1, min_w = 0, gap = 8, items = "center", cover(22, 6), white(song("title"), "$font.size.xs") }
	end
	return ui.row { grow = 1, min_w = 0, gap = 8, items = "center", timer_ring(18, 2.5), amber(timer_text, "$font.size.sm") }
end
local function badge(kind: string): Node
	if kind == "agents" then
		return ui.row {
			gap = 5,
			items = "center",
			kx.Spinner { size = 11 },
			ui.text { function()
				return tostring(#working:get())
			end, literal = true, text_size = "$font.size.xs", weight = "$font.weight.semibold", color = "#a9c8ff", tabular = true },
		}
	elseif kind == "music" then
		return ui.row { gap = 5, items = "center", cover(16, 4), waveform(4, 12, 2) }
	end
	return ui.row { gap = 4, items = "center", timer_ring(13, 2), amber(timer_text) }
end
local function slim_many(kind: string): Node
	local kinds = string.split(kind, "+")
	local row: { [any]: any } = { w_full = true, h_full = true, pl = 12, pr = 6, gap = 6, items = "center", lead_of(kinds[1]) }
	for i = 2, #kinds do
		table.insert(row, ui.row {
			h = 24,
			px = 8,
			shrink = 0,
			items = "center",
			rounded = "$radius.full",
			bg = ui.raw("#ffffff12", "a live activity's chip"),
			border = 1,
			border_color = ui.raw("#ffffff0f", "a live activity's chip, its rim"),
			badge(kinds[i]),
		})
	end
	return ui.row(row)
end

local slim_music = compact(cover(22, 6), white(song("title"), "$font.size.xs"), waveform(5, 14, 2))

local function slim(): Node
	return ui.presence(slim_kind, function(kind: string): Node
		if kind == "waiting" then
			return ui.row {
				w_full = true,
				h_full = true,
				px = 16,
				items = "center",
				asking(function()
					return first:get()
				end, true),
			}
		end
		if kind == "agents" then
			return slim_working
		elseif kind == "music" then
			return slim_music
		elseif kind == "timer" then
			return slim_timer
		elseif kind == "resting" then
			return slim_resting
		end
		return slim_many(kind)
	end, { w_full = true, h_full = true, enter = { opacity = { 0, 1 }, scale = { 0.94, 1 }, duration = 200 }, exit = { opacity = { 1, 0 }, duration = 80 } })
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

local function progress(): Node
	local function fraction(): number
		local m = media:get()
		return if m and m.duration > 0 then math.clamp(position() / m.duration, 0, 1) else 0
	end
	local function stamp(text: any, align: string?): Node
		return ui.text {
			text,
			literal = true,
			text_size = ui.raw(11, "the track's time"),
			weight = "$font.weight.medium",
			color = "#ffffffc4",
			tabular = true,
			align = align,
			shrink = 0,
		}
	end
	return ui.row {
		gap = ui.raw(10, "space round the time"),
		items = "center",
		w_full = true,
		stamp(function()
			return mmss(position())
		end),
		ui.column {
			h = 18,
			grow = 1,
			justify = "center",
			on_click = function(e: any)
				local m = media:peek()
				if m and m.duration > 0 and e and e.local_x and e.width and e.width > 0 then
					control({ seek = math.clamp(e.local_x / e.width, 0, 1) })
				end
			end,
			ui.div {
				h = 3,
				w_full = true,
				rounded = "$radius.full",
				bg = ui.raw("#ffffff38", "the track still to play"),
				ui.div {
					h_full = true,
					rounded = "$radius.full",
					bg = ui.raw("#fffffff2", "how far the track has played"),
					relative = true,
					w = function()
						return `{math.floor(fraction() * 1000) / 10}%`
					end,
					ui.div {
						absolute = true,
						right = -5,
						top = -3,
						w = 9,
						h = 9,
						rounded = "$radius.full",
						bg = ui.raw("#ffffff", "the knob on the progress line"),
						shadow = ui.raw({ x = 0, y = 1, blur = 5, spread = 0, color = "#00000059" }, "the knob's shadow"),
					},
				},
			},
		},
		stamp(function()
			local m = media:get()
			return if m and m.duration > 0 then `-{mmss(math.max(0, m.duration - position()))}` else "0:00"
		end, "right"),
	}
end

local function play_key(size: number, bare: boolean?): Node
	return ui.presence(function(): string
		local m = media:get()
		return if m and m.playing then "pause" else "play"
	end, function(k: string): Node
		return key(if k == "pause" then "media-pause" else "media-play", "play", if k == "pause" then "Pause" else "Play", size, bare)
	end, { enter = { opacity = { 0, 1 }, scale = { 0.7, 1 }, duration = 180 }, exit = { opacity = 0, duration = 60 }, mode = "wait" })
end

-- A well of glass on the island's glass, every card the same: a pane a
-- touch lighter at its top, a hairline rim and a lit top lip, so the
-- cards read as set into the glass rather than flat grey patches.
local WELL_FILL = { angle = 180, stops = { { 0, ui.raw("#ffffff17", "a glass well's lit top") }, { 1, ui.raw("#ffffff08", "a glass well's foot") } } }
local function well(p: { [any]: any }): { [any]: any }
	p.bg = nil
	p.gradient = WELL_FILL
	p.border = 1
	p.border_color = ui.raw("#ffffff14", "a glass well's rim")
	p.inset_shadow = { x = 0, y = 1, blur = 0, spread = 0, color = ui.raw("#ffffff12", "a glass well's lit lip") }
	return p
end

local function music_row(big: boolean): Node
	if not big then
		return ui.row(well {
			gap = "$space.3",
			items = "center",
			w_full = true,
			p = "$space.2",
			rounded = 18,
			cover(40, 10),
			ui.column {
				grow = 1,
				min_w = 0,
				white(song("title"), "$font.size.sm", "$font.weight.semibold"),
				quiet(song("by"), "$font.size.xs"),
			},
			waveform(7, 18, 2),
			key("media-previous", "previous", "Previous", 14),
			play_key(16),
			key("media-next", "next", "Next", 14),
		})
	end
	return ui.column {
		w_full = true,
		h_full = true,
		justify = "between",
		-- Large cover with its own light behind it, the track in capitals,
		-- where it plays from, and the waveform in the cover's colours.
		ui.column {
			w_full = true,
			gap = "$space.4",
			ui.row {
				gap = "$space.4",
				items = "center",
				w_full = true,
				ui.div {
					w = 76,
					h = 76,
					shrink = 0,
					ui.div {
						absolute = true,
						left = 5,
						top = 9,
						w = 66,
						h = 66,
						rounded = ui.raw(16, "the cover glow's corner"),
						bg = function()
							return ui.raw(mix(tint(), "#000000", 0.08) .. "59", "light from the cover, behind it")
						end,
					},
					cover(76, 16),
				},
				ui.column {
					grow = 1,
					min_w = 0,
					gap = 4,
					ui.text {
						function()
							local m = media:get()
							if not m or m.title == "" then
								return "Nothing playing"
							end
							return string.upper(m.title)
						end,
						literal = true,
						truncate = true,
						color = "#ffffff",
						text_size = ui.raw(17, "the track, said large"),
						weight = "$font.weight.bold",
						letter_spacing = 1,
					},
					ui.text {
						song("by"),
						literal = true,
						truncate = true,
						text_size = ui.raw(13, "the artist"),
						weight = "$font.weight.medium",
						color = ui.raw("#ffffffc2", "the artist, under the title"),
						letter_spacing = 0.3,
					},
					ui.row {
						gap = 6,
						items = "center",
						ui.div {
							w = 5,
							h = 5,
							rounded = "$radius.full",
							bg = function()
								local m = media:get()
								return if m and m.playing then ui.raw(mix("#ffffff", tint(), 0.45), "the playing light") else "#ffffff66"
							end,
						},
						ui.text {
							function()
								local m = media:get()
								if not m then
									return "Windows media"
								end
								return if m.playing then `Playing · {m.app}` else `Paused · {m.app}`
							end,
							literal = true,
							truncate = true,
							text_size = ui.raw(10, "where the track plays"),
							weight = "$font.weight.semibold",
							color = "#ffffff7a",
							letter_spacing = 0.7,
						},
					},
				},
				waveform(9, 42, 3),
			},
			progress(),
		},
		-- The transport, large and bare, low in the glass.
		ui.row {
			gap = ui.raw(22, "space between the transport keys"),
			items = "center",
			justify = "center",
			w_full = true,
			pb = ui.raw(2, "the transport's lift from the foot"),
			key("media-rewind", "previous", "Previous", 30, true),
			play_key(40, true),
			key("media-forward", "next", "Next", 30, true),
		},
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
local some_threads = threads(3)
-- (Two while music plays, so Home keeps within the island's window.)
local home_threads = ui.computed(function(): { store.Thread }
	local list = some_threads:get()
	local n = if media:get() ~= nil then 2 else 3
	if #list <= n then
		return list
	end
	return { table.unpack(list, 1, n) }
end)
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

-- A tile on the island: a flat well a shade lighter than the black, its
-- corner concentric with the island's (38 less the 14 around it).
local function tile(props: { [any]: any }): Node
	local p: { [any]: any } = well {
		rounded = 24,
		p = 14,
		gap = "$space.2",
	}
	for k, v in props do
		p[k] = v
	end
	return ui.column(p)
end

local function label(text: string | () -> string): Node
	return ui.text {
		function()
			return string.upper(if type(text) == "function" then text() else text)
		end,
		literal = true,
		text_size = ui.raw(11, "a tile's label"),
		weight = "$font.weight.semibold",
		letter_spacing = 0.6,
		color = "#ffffff80",
	}
end

local function grey(text: any, size: string?): Node
	return ui.text { text, literal = true, truncate = true, text_size = size or "$font.size.xs", color = "#ffffff8c" }
end

-- A page's heading: its name in small capitals, a quiet count after it,
-- and what acts on the whole page at its end.
local function section(title: string | () -> string, count: (() -> string)?, trailing: Node?): Node
	return ui.row {
		w_full = true,
		h = 28,
		px = 4,
		gap = 6,
		items = "center",
		label(title),
		if count
			then ui.text { count, literal = true, text_size = ui.raw(12, "a heading's count"), color = "#ffffff4d", tabular = true }
			else nil,
		ui.row { grow = 1 },
		trailing,
	}
end

-- A small word on a coloured wash: how something stands.
local function chip(text: any, colour: string | () -> string): Node
	local function c(): string
		return if type(colour) == "function" then colour() else colour
	end
	-- (A soft wash of its colour, a dot in it, and the word.)
	return ui.row {
		h = 22,
		px = 8,
		gap = 6,
		items = "center",
		rounded = "$radius.full",
		bg = function()
			return ui.raw(c() .. "1f", "a chip's wash")
		end,
		border = 1,
		border_color = function()
			return ui.raw(c() .. "33", "a chip's rim")
		end,
		ui.div { w = 6, h = 6, rounded = "$radius.full", bg = c },
		ui.text { text, literal = true, text_size = ui.raw(12, "a chip's word"), weight = "$font.weight.medium", color = "#ffffffa6" },
	}
end

-- Nothing here yet: a mark in a ring of light, what this page is for, and
-- how to fill it.
local function empty_state(icon: string, title: string, body: string, action: Node?): Node
	return ui.column {
		w_full = true,
		h_full = true,
		items = "center",
		justify = "center",
		gap = 10,
		ui.column {
			w = 56,
			h = 56,
			shrink = 0,
			rounded = "$radius.full",
			items = "center",
			justify = "center",
			color = "#ffffffa6",
			gradient = { angle = 160, stops = { { 0, ui.raw("#ffffff21", "an empty page's ring, lit") }, { 1, ui.raw("#ffffff05", "an empty page's ring, its foot") } } },
			border = 1,
			border_color = ui.raw("#ffffff1f", "an empty page's ring, its rim"),
			inset_shadow = { x = 0, y = 1, blur = 0, spread = 0, color = ui.raw("#ffffff1f", "an empty page's ring, its lip") },
			kx.Icon { name = icon, size = 22 },
		},
		ui.column {
			gap = 3,
			items = "center",
			max_w = 300,
			white(title, "$font.size.sm", "$font.weight.semibold"),
			ui.text { body, literal = true, align = "center", text_size = ui.raw(12, "an empty page's line"), color = "#ffffff80" },
		},
		action,
	}
end

-- An agent at a glance: its mark, its thread, how it stands.
local function agent_line(t: store.Thread): Node
	local waits: Waiting? = nil
	for _, w in attention:peek() do
		if w.thread_id == t.id then
			waits = w
		end
	end
	local ring = if waits then "#ffb454" elseif t.status == "running" then "#8fb4ff" elseif t.status == "failed" then "#ff7a7a" else "#ffffff"
	return ui.button {
		w_full = true,
		h = 46,
		gap = "$space.3",
		items = "center",
		px = 8,
		rounded = 16,
		bg = "#00000000",
		hover = { bg = "#ffffff12" },
		active = { scale = 0.985 },
		tooltip = "Open the chat",
		on_click = function()
			open_chat(t.id)
		end,
		-- Its mark on a square lit in how it stands: warm waiting, blue
		-- working (breathing), red stopped, quiet done.
		ui.column {
			w = 30,
			h = 30,
			shrink = 0,
			rounded = 10,
			items = "center",
			justify = "center",
			bg = ui.raw(if ring == "#ffffff" then "#ffffff0f" else ring .. "1f", "an agent's square, in how it stands"),
			border = 1,
			border_color = ui.raw(if ring == "#ffffff" then "#ffffff14" else ring .. "59", "an agent's square's rim"),
			shadow = if waits or t.status == "running"
				then { x = 0, y = 0, blur = 8, spread = -2, color = ui.raw(ring .. "59", "an agent's square's glow") }
				else nil,
			kx.AgentMark { agent = t.harness_id, size = 16 },
		},
		ui.column {
			grow = 1,
			min_w = 0,
			gap = 1,
			ui.text { t.title, literal = true, truncate = true, text_size = "$font.size.sm", weight = "$font.weight.semibold", color = "#fffffff0" },
			ui.text {
				if waits then `Wants to run {waits.tool}`
					elseif t.status == "running" then `{t.harness} · working`
					elseif t.status == "failed" then `{t.harness} · stopped with an error`
					else `{t.harness} · done`,
				literal = true,
				truncate = true,
				text_size = ui.raw(11, "a row's second line"),
				color = if waits then "$kx.attention" elseif t.status == "failed" then ui.raw("#ff9a9acc", "a failed row's line") else "#ffffff73",
			},
		},
		if waits
			then ui.row {
				gap = 4,
				pill("Deny", false, function()
					decide(waits, "deny")
				end),
				pill("Allow", true, function()
					decide(waits, "allow_once")
				end),
			}
			elseif t.status == "running" then kx.Spinner { size = 13 }
			else ui.column { color = "#ffffff4d", kx.Icon { name = "alt-arrow-right", size = 13 } },
	}
end

-- The ask: a pill to type in, and a round key to send.
local function ask_pill(): Node
	return ui.row(well {
		w_full = true,
		h = 44,
		gap = "$space.2",
		items = "center",
		pl = 16,
		pr = 5,
		shrink = 0,
		rounded = "$radius.full",
		ui.input {
			value = ask_text,
			placeholder = "Ask an agent",
			placeholder_color = "#ffffff59",
			label = "Ask an agent",
			grow = 1,
			min_w = 0,
			h = 34,
			bg = "#00000000",
			border = 0,
			color = "#ffffffeb",
			text_size = "$font.size.sm",
			on_key = function(e: any): boolean?
				if e.key == "Enter" then
					ask()
					return true
				end
				return false
			end,
		},
		ui.button {
			w = 34,
			h = 34,
			rounded = "$radius.full",
			items = "center",
			justify = "center",
			bg = function()
				return if string.match(ask_text:get(), "^%s*$") then "#ffffff1f" else "#ffffff"
			end,
			hover = { scale = 1.05 },
			active = { scale = 0.92 },
			color = function()
				return if string.match(ask_text:get(), "^%s*$") then "#ffffff80" else "#000000"
			end,
			label = "Ask",
			on_click = ask,
			ui.show(ask_busy, kx.Spinner { size = 12 }, kx.Icon { name = "arrow-up", size = 15 }),
		},
	})
end

local has_media = ui.computed(function(): boolean
	return media:get() ~= nil
end)

-- What's playing, as a live activity: the cover's light washing the card,
-- a click on it opens the player.
local function now_playing(): Node
	return ui.column(well {
		w_full = true,
		h = 64,
		shrink = 0,
		rounded = 24,
		overflow = "hidden",
		ui.row {
			w_full = true,
			h_full = true,
			pl = 10,
			pr = 6,
			gap = "$space.3",
			items = "center",
			ui.button {
				grow = 1,
				min_w = 0,
				h_full = true,
				gap = "$space.3",
				items = "center",
				bg = "#00000000",
				label = "Open the player",
				on_click = function()
					tab:set("music")
				end,
				cover(44, 11),
				ui.column {
					grow = 1,
					min_w = 0,
					gap = 2,
					white(song("title"), "$font.size.sm", "$font.weight.semibold"),
					ui.text { song("by"), literal = true, truncate = true, text_size = ui.raw(11, "the artist"), color = "#ffffff99" },
				},
				waveform(7, 20, 2),
			},
			ui.row {
				gap = 0,
				items = "center",
				key("media-rewind", "previous", "Previous", 15, true),
				play_key(19, true),
				key("media-forward", "next", "Next", 15, true),
			},
		},
	})
end

-- A running timer on Home, as a live activity: its ring, what's left and
-- when it ends, and a minute more or stop.
local function timer_card(): Node
	return ui.row(well {
		w_full = true,
		h = 54,
		shrink = 0,
		rounded = 24,
		pl = 12,
		pr = 8,
		gap = "$space.3",
		items = "center",
		ui.button {
			grow = 1,
			min_w = 0,
			h_full = true,
			gap = "$space.3",
			items = "center",
			bg = "#00000000",
			label = "Open the timer",
			on_click = function()
				tab:set("timer")
			end,
			timer_ring(30, 3.5),
			ui.column {
				grow = 1,
				min_w = 0,
				gap = 1,
				amber(timer_text, "$font.size.md"),
				ui.text { function()
					return `Ends {os.date("%H:%M", timer_end:get())}`
				end, literal = true, text_size = ui.raw(11, "when the timer ends"), color = "#ffffff99" },
			},
		},
		pill("+1 min", false, add_minute),
	})
end

-- How things stand, said once: who waits, who works, or all quiet.
local function standing(): Node
	return ui.presence(function(): string
		if #attention:get() > 0 then
			return "waiting"
		end
		return if #working:get() > 0 then "working" else "quiet"
	end, function(k: string): Node
		if k == "waiting" then
			return chip(function()
				local n = #attention:get()
				return if n == 1 then "1 needs you" else `{n} need you`
			end, "#ffb454")
		elseif k == "working" then
			return chip(function()
				local n = #working:get()
				return if n == 1 then "1 working" else `{n} working`
			end, "#8fb4ff")
		end
		return chip("All quiet", "#7ee0a8")
	end, { enter = { opacity = { 0, 1 }, scale = { 0.9, 1 }, duration = 180 }, exit = { opacity = 0, duration = 60 }, mode = "wait" })
end

local function home(): Node
	return ui.column {
		gap = 10,
		w_full = true,
		-- The time, large, the day beside it, and how things stand.
		ui.row {
			w_full = true,
			items = "end",
			gap = "$space.3",
			px = 4,
			h = 48,
			shrink = 0,
			ui.text { clock, literal = true, text_size = ui.raw(40, "the island's clock"), weight = "$font.weight.semibold", color = "#ffffff", tabular = true, letter_spacing = -1 },
			ui.text { function()
				local t = now:get()
				return `{t.weekday}, {t.day} {t.month}`
			end, literal = true, pb = 7, text_size = "$font.size.sm", weight = "$font.weight.medium", color = "#ffffff8c" },
			ui.row { grow = 1 },
			ui.row { pb = 8, standing() },
		},
		ui.show(has_media, now_playing()),
		ui.show(timer_on, timer_card()),
		-- Your agents, each a live activity: click one to talk to it.
		tile {
			w_full = true,
			gap = 0,
			p = 6,
			shrink = 0,
			section("Agents", function()
				return tostring(#store.threads:get())
			end, ui.button {
				h = 22,
				px = 8,
				gap = 3,
				items = "center",
				rounded = "$radius.full",
				bg = "#00000000",
				hover = { bg = "#ffffff14" },
				color = "#ffffff99",
				label = "See all agents",
				on_click = function()
					tab:set("agents")
				end,
				ui.text { "See all", literal = true, text_size = ui.raw(11, "a heading's link"), weight = "$font.weight.medium" },
				kx.Icon { name = "alt-arrow-right", size = 11 },
			}),
			ui.show(function()
				return #home_threads:get() > 0
			end, ui.column {
				w_full = true,
				ui.each(home_threads, agent_line, {
					key = function(t: store.Thread)
						return `{t.id}:{t.status}`
					end,
					gap = 0,
				}),
			}, ui.row {
				w_full = true,
				h = 46,
				px = 10,
				gap = "$space.2",
				items = "center",
				ui.column { color = "#ffffff59", kx.Icon { name = "bot", size = 15 } },
				grey("No agents yet. Ask one below.", "$font.size.sm"),
			}),
		},
		ask_pill(),
	}
end

-- ── The chat ───────────────────────────────────────────────────────────

local reply_text = ui.state("notch.reply", "")
local reply_busy = ui.state("notch.reply.busy", false)
local function reply()
	local text = reply_text:peek()
	if string.match(text, "^%s*$") or reply_busy:peek() then
		return
	end
	reply_busy:set(true)
	store.send(text):after(function()
		reply_text:set("")
		reply_busy:set(false)
	end, function(why: any)
		reply_busy:set(false)
		kx.toast { title = "Couldn't send", body = tostring(if type(why) == "table" then why.message else why), kind = "error" }
	end)
end

local function chat_thread(): store.Thread?
	local id = chat:get()
	for _, t in store.threads:get() do
		if t.id == id then
			return t
		end
	end
	return nil
end

local function round_button(icon: string, name: string, run: () -> (), strong: boolean?): Node
	return ui.button {
		w = 30,
		h = 30,
		rounded = "$radius.full",
		items = "center",
		justify = "center",
		bg = if strong then "#ffffff" else "#ffffff14",
		hover = { bg = if strong then "#ffffffd9" else "#ffffff26" },
		active = { scale = 0.92 },
		color = if strong then "#000000" else "#ffffffcc",
		label = name,
		tooltip = name,
		on_click = run,
		kx.Icon { name = icon, size = 14 },
	}
end

local function chat_view(): Node
	local running = ui.computed(function(): boolean
		local t = store.thread:get()
		return t ~= nil and t.running
	end)
	return ui.column {
		w_full = true,
		h_full = true,
		gap = 10,
		-- Back, the agent and its thread, and the thread in the window.
		ui.row {
			w_full = true,
			gap = "$space.3",
			items = "center",
			round_button("alt-arrow-left", "Back", function()
				chat:set(nil)
			end),
			ui.column {
				w = 34,
				h = 34,
				shrink = 0,
				rounded = "$radius.full",
				items = "center",
				justify = "center",
				bg = function()
					return if running:get() then ui.raw("#8fb4ff1f", "a working agent's disc") else "#ffffff0f"
				end,
				border = 1,
				border_color = function()
					return if running:get() then ui.raw("#8fb4ff4d", "a working agent's ring") else "#ffffff14"
				end,
				ui.presence(function(): string
					local t = chat_thread()
					return if t then t.harness_id else ""
				end, function(agent: string): Node
					return kx.AgentMark { agent = agent, size = 18 }
				end),
			},
			ui.column {
				grow = 1,
				min_w = 0,
				gap = 0,
				white(function()
					local t = chat_thread()
					return if t then t.title else ""
				end, "$font.size.sm", "$font.weight.semibold"),
				grey(function()
					local o = store.thread:get()
					local t = chat_thread()
					if o and o.running then
						return if o.status ~= "" then o.status else "Working…"
					end
					return if t then t.harness else ""
				end),
			},
			ui.show(running, kx.Spinner { size = 14 }),
			round_button("arrow-up-right", "Open in Keplorayx", function()
				open_thread(chat:peek())
			end),
		},
		-- The conversation.
		ui.column {
			w_full = true,
			grow = 1,
			min_h = 0,
			rounded = 22,
			bg = "#0000004d",
			border = 1,
			border_color = "#ffffff14",
			inset_shadow = { x = 0, y = 1, blur = 0, spread = 0, color = "#ffffff0f" },
			overflow = "hidden",
			-- (The window's transcript, read at the island's size.)
			zoom = 0.84,
			ui.component(function(): Node
				return Transcript {
					messages = conversation.messages,
					draft = conversation.draft,
					compact = function(): boolean
						return true
					end,
					measure = function(): number
						return 500
					end,
				}
			end)({}),
		},
		-- What it waits on, answered here.
		ui.show(function()
			return store.approval:get() ~= nil
		end, ui.row {
			w_full = true,
			gap = "$space.2",
			items = "center",
			px = 12,
			py = 8,
			rounded = 18,
			bg = "#ffb4541f",
			border = 1,
			border_color = "#ffb45440",
			ui.div { w = 7, h = 7, rounded = "$radius.full", bg = "$kx.attention", opacity = glow, shrink = 0 },
			ui.text {
				function()
					local a = store.approval:get()
					return if a then `Wants to run {a.tool}{if a.summary ~= "" then ": " .. a.summary else ""}` else ""
				end,
				literal = true,
				truncate = true,
				grow = 1,
				min_w = 0,
				text_size = "$font.size.xs",
				color = "#ffffffe6",
			},
			pill("Deny", false, function()
				local a = store.approval:peek()
				if a then
					store.decide(a.request_id, "deny")
				end
			end),
			pill("Allow", true, function()
				local a = store.approval:peek()
				if a then
					store.decide(a.request_id, "allow_once")
				end
			end),
		}),
		-- Your reply (while it works, it goes after; stop is beside it).
		ui.row {
			w_full = true,
			h = 44,
			gap = "$space.2",
			items = "center",
			pl = 16,
			pr = 5,
			shrink = 0,
			rounded = "$radius.full",
			gradient = { angle = 180, stops = { { 0, "#ffffff1a" }, { 1, "#ffffff0d" } } },
			border = 1,
			border_color = "#ffffff1a",
			inset_shadow = { x = 0, y = 1, blur = 0, spread = 0, color = "#ffffff1a" },
			ui.input {
				value = reply_text,
				placeholder = "Reply…",
				placeholder_color = "#ffffff59",
				label = "Reply",
				grow = 1,
				min_w = 0,
				h = 34,
				bg = "#00000000",
				border = 0,
				color = "#ffffffeb",
				text_size = "$font.size.sm",
				on_key = function(e: any): boolean?
					if e.key == "Enter" and not e.shift then
						reply()
						return true
					end
					return false
				end,
			},
			ui.show(running, round_button("stop", "Stop", function()
				store.interrupt()
			end)),
			ui.button {
				w = 34,
				h = 34,
				rounded = "$radius.full",
				items = "center",
				justify = "center",
				bg = function()
					return if string.match(reply_text:get(), "^%s*$") then "#ffffff1f" else "#ffffff"
				end,
				hover = { scale = 1.05 },
				active = { scale = 0.92 },
				color = function()
					return if string.match(reply_text:get(), "^%s*$") then "#ffffff80" else "#000000"
				end,
				label = "Send",
				on_click = reply,
				ui.show(reply_busy, kx.Spinner { size = 12 }, kx.Icon { name = "arrow-up", size = 15 }),
			},
		},
	}
end

local note = if fixture then ui.state("notch.note.fx", "Ship the island before the demo.") else ui.persist("notch.note", "")

local function timer(): Node
	local presets: { [any]: any } = { gap = "$space.2", items = "center" }
	for _, m in { 5, 15, 25, 50 } do
		table.insert(presets, ui.button {
			h = 32,
			px = 14,
			rounded = "$radius.full",
			items = "center",
			bg = "#ffffff12",
			border = 1,
			border_color = "#ffffff14",
			hover = { bg = "#ffffff24" },
			active = { scale = 0.94 },
			color = "#ffffffe6",
			label = `Start {m} minutes`,
			on_click = function()
				start_timer(m)
			end,
			ui.text { `{m}`, literal = true, text_size = "$font.size.sm", weight = "$font.weight.semibold", tabular = true },
			ui.text { " min", literal = true, text_size = ui.raw(11, "the unit"), color = "#ffffff80" },
		})
	end
	local RING = 156
	return ui.column {
		gap = "$space.4",
		w_full = true,
		h_full = true,
		items = "center",
		justify = "center",
		-- The time left in a ring that empties as it runs.
		ui.div {
			w = RING,
			h = RING,
			shrink = 0,
			ui.canvas {
				absolute = true,
				left = 0,
				top = 0,
				w = RING,
				h = RING,
				draw = function(c, w, h)
					local cx, cy, r = w / 2, h / 2, math.min(w, h) / 2 - 6
					c:stroke(ui.path():arc(cx, cy, r, 0, 360), "#ffffff1a", 6)
					local total = timer_total:get()
					local left = timer_left:get()
					if total > 0 and left > 0 then
						local f = math.clamp(left / total, 0, 1)
						c:stroke(ui.path():arc(cx, cy, r, -90, 360 * f), "#ffb454", 6, { cap = "round" })
						local a = math.rad(-90 + 360 * f)
						c:circle(cx + math.cos(a) * r, cy + math.sin(a) * r, 5, "#ffffff")
					end
				end,
			},
			ui.column {
				absolute = true,
				left = 0,
				top = 0,
				w = RING,
				h = RING,
				items = "center",
				justify = "center",
				gap = 0,
				ui.text {
					function()
						local l = timer_left:get()
						return string.format("%d:%02d", math.floor(l / 60), l % 60)
					end,
					literal = true,
					font = "$kx.display",
					text_size = ui.raw(44, "the timer's figures"),
					color = function()
						return if timer_left:get() > 0 then "#ffffff" else "#ffffff59"
					end,
					tabular = true,
				},
				label(function()
					if timer_left:get() <= 0 then
						return "Focus timer"
					end
					local e = timer_end:get()
					return `Ends {os.date("%H:%M", e)}`
				end),
			},
		},
		ui.presence(function(): string
			return if timer_left:get() > 0 then "running" else "idle"
		end, function(k: string): Node
			if k == "running" then
				return ui.row {
					gap = "$space.2",
					items = "center",
					pill("+1 min", false, add_minute),
					pill("Stop", true, function()
						timer_end:set(0)
						timer_left:set(0)
					end),
				}
			end
			return ui.row(presets)
		end, { enter = { opacity = { 0, 1 }, y = { 4, 0 }, duration = 160 }, exit = { opacity = 0, duration = 60 }, mode = "wait" }),
	}
end

-- The tray (Dove's): files dropped on the island, kept until you take them
-- away; each opened, or shown in its folder.
type Kept = { path: string, name: string }
local tray = if fixture
	then ui.state("notch.tray.fx", { { path = "C:/Users/you/Desktop/brief.pdf", name = "brief.pdf" }, { path = "C:/Users/you/Downloads/shot.png", name = "shot.png" } } :: { Kept })
	else ui.persist("notch.tray", {} :: { Kept })
local function keep(files: { any })
	local list = table.clone(tray:peek())
	for _, f in files or {} do
		local path = f.path or ""
		if path ~= "" then
			local here = false
			for _, k in list do
				if k.path == path then
					here = true
				end
			end
			if not here then
				table.insert(list, 1, { path = path, name = f.name or string.match(path, "[^/\\]+$") or path })
			end
		end
	end
	tray:set(list)
end

local function ago(at: number): string
	local d = os.time() - at
	if d < 60 then
		return "now"
	elseif d < 3600 then
		return `{math.floor(d / 60)}m`
	elseif d < 86400 then
		return `{math.floor(d / 3600)}h`
	end
	return `{math.floor(d / 86400)}d`
end

-- A file's kind: its glyph and its colour.
local function kind_of(name: string): (string, string, string)
	local ext = string.lower(string.match(name, "%.([%w]+)$") or "")
	if ext == "png" or ext == "jpg" or ext == "jpeg" or ext == "gif" or ext == "webp" or ext == "svg" or ext == "heic" then
		return "file-image", "#c39bff", ext
	elseif ext == "pdf" then
		return "document", "#ff8a7a", ext
	elseif ext == "zip" or ext == "7z" or ext == "rar" or ext == "tar" or ext == "gz" then
		return "archive-minimalistic", "#ffc46b", ext
	elseif ext == "md" or ext == "txt" then
		return "file-markdown", "#a7b4c8", ext
	elseif ext == "json" or ext == "csv" or ext == "xlsx" or ext == "sql" then
		return "file-data", "#7ee0a8", ext
	elseif ext == "rs" or ext == "ts" or ext == "tsx" or ext == "js" or ext == "py" or ext == "luau" or ext == "lua" or ext == "go" or ext == "html" or ext == "css" then
		return "file-code", "#8fb4ff", ext
	end
	return "document", "#d0d4dc", if ext == "" then "file" else ext
end

local function untray(path: string)
	local list = {}
	for _, x in tray:peek() do
		if x.path ~= path then
			table.insert(list, x)
		end
	end
	tray:set(list)
end

local function little(icon: string, name: string, run: () -> ()): Node
	return ui.button {
		w = 24,
		h = 24,
		rounded = "$radius.full",
		items = "center",
		justify = "center",
		bg = "#00000000",
		hover = { bg = "#ffffff1f" },
		active = { scale = 0.9 },
		color = "#ffffff99",
		label = name,
		tooltip = name,
		on_click = run,
		kx.Icon { name = icon, size = 12 },
	}
end

local function tray_page(): Node
	local function file_tile(k: Kept): Node
		local icon, hue, ext = kind_of(k.name)
		return ui.column {
			w = 100,
			gap = 6,
			items = "center",
			ui.button {
				w = 100,
				h = 78,
				rounded = 18,
				gradient = { angle = 160, stops = { { 0, ui.raw(hue .. "2b", "a file's colour, washed") }, { 1, ui.raw("#ffffff0a", "a file tile's foot") } } },
				border = 1,
				border_color = ui.raw(hue .. "33", "a file tile's rim"),
				inset_shadow = { x = 0, y = 1, blur = 0, spread = 0, color = "#ffffff1a" },
				hover = { scale = 1.03 },
				active = { scale = 0.97 },
				direction = "column",
				items = "center",
				justify = "center",
				gap = 7,
				label = `Open {k.name}`,
				tooltip = k.path,
				on_click = function()
					pcall(ui.invoke, "open_url", k.path)
				end,
				ui.column { color = ui.raw(hue, "a file's glyph"), kx.Icon { name = icon, size = 26 } },
				ui.row {
					h = 16,
					px = 6,
					items = "center",
					rounded = "$radius.full",
					bg = "#00000059",
					ui.text { string.upper(string.sub(ext, 1, 5)), literal = true, text_size = ui.raw(9, "a file's kind"), weight = "$font.weight.bold", color = "#ffffffcc", letter_spacing = 0.5 },
				},
			},
			ui.text { k.name, literal = true, truncate = true, max_w = 100, text_size = ui.raw(11, "a file's name"), weight = "$font.weight.medium", color = "#ffffffd9" },
			ui.row {
				gap = 2,
				little("folder", "Show in folder", function()
					pcall(ui.invoke, "notch.reveal", k.path)
				end),
				little("close", "Take it away", function()
					untray(k.path)
				end),
			},
		}
	end
	return ui.show(
		function()
			return #tray:get() > 0
		end,
		ui.column {
			w_full = true,
			h_full = true,
			gap = 8,
			section("Tray", function()
				local n = #tray:get()
				return if n == 1 then "1 file" else `{n} files`
			end, pill("Clear", false, function()
				tray:set({})
			end)),
			ui.scroll {
				w_full = true,
				grow = 1,
				min_h = 0,
				ui.row {
					w_full = true,
					wrap = true,
					gap = 10,
					ui.each(tray, file_tile, {
						key = function(k: Kept)
							return k.path
						end,
					}),
				},
			},
		},
		ui.column {
			w_full = true,
			h_full = true,
			rounded = 24,
			border = 1,
			border_color = "#ffffff26",
			border_style = "dashed",
			bg = "#ffffff05",
			empty_state("download", "Drop files on the island", "Drag anything to the top of the screen. It waits here until you take it away."),
		}
	)
end

local function alert_row(a: Alert): Node
	local hue = if a.kind == "failed" then "#ff7a7a" elseif a.kind == "asked" then "#ffb454" else "#7ee0a8"
	local word = if a.kind == "failed" then "Failed" elseif a.kind == "asked" then "Asked you" else "Finished"
	return ui.button {
		w_full = true,
		h = 46,
		gap = "$space.3",
		items = "center",
		px = 8,
		rounded = 16,
		bg = "#00000000",
		hover = { bg = "#ffffff12" },
		active = { scale = 0.985 },
		on_click = function()
			open_chat(a.thread)
		end,
		ui.column {
			w = 30,
			h = 30,
			shrink = 0,
			rounded = "$radius.full",
			items = "center",
			justify = "center",
			bg = ui.raw(hue .. "1f", "an alert's disc"),
			border = 1,
			border_color = ui.raw(hue .. "40", "an alert's ring"),
			kx.StatusIcon { kind = if a.kind == "failed" then "failed" elseif a.kind == "asked" then "approval" else "done", size = 14 },
		},
		ui.column {
			grow = 1,
			min_w = 0,
			gap = 1,
			ui.text { a.title, literal = true, truncate = true, text_size = "$font.size.sm", weight = "$font.weight.medium", color = "#ffffffeb" },
			ui.text { word, literal = true, text_size = ui.raw(11, "what happened"), weight = "$font.weight.medium", color = ui.raw(hue .. "cc", "what happened, in its colour") },
		},
		ui.text { ago(a.at), literal = true, text_size = ui.raw(11, "how long ago"), color = "#ffffff59", tabular = true },
	}
end

local function alerts_page(): Node
	return ui.show(
		function()
			return #alerts:get() > 0
		end,
		ui.column {
			w_full = true,
			h_full = true,
			gap = 6,
			section("Recent", function()
				return tostring(#alerts:get())
			end, pill("Clear", false, function()
				alerts:set({})
			end)),
			tile {
				w_full = true,
				grow = 1,
				min_h = 0,
				p = 4,
				gap = 0,
				ui.scroll {
					w_full = true,
					h_full = true,
					ui.column {
						w_full = true,
						gap = 0,
						ui.each(alerts, alert_row, {
							key = function(a: Alert)
								return `{a.thread}:{a.at}:{a.kind}`
							end,
						}),
					},
				},
			},
		},
		empty_state("bell", "All quiet", "When an agent finishes, fails or asks you something, it lands here.")
	)
end

local function notes_page(): Node
	return ui.column {
		w_full = true,
		h_full = true,
		gap = 6,
		section("Note", nil, ui.row {
			gap = 6,
			items = "center",
			ui.text {
				function()
					local n = #note:get()
					return if n == 0 then "Saved as you type" else `{n} characters · saved`
				end,
				literal = true,
				text_size = ui.raw(11, "the note's state"),
				color = "#ffffff59",
				tabular = true,
			},
			little("trash-bin-minimalistic", "Clear the note", function()
				note:set("")
			end),
		}),
		tile {
			w_full = true,
			grow = 1,
			min_h = 0,
			p = 4,
			bg = "#0000003d",
			ui.textarea {
				value = note,
				placeholder = "Jot something down. It stays here until you clear it…",
				placeholder_color = "#ffffff4d",
				w_full = true,
				grow = 1,
				p = 12,
				color = "#fffffff0",
				text_size = "$font.size.sm",
				bg = "#00000000",
				border = 0,
			},
		},
	}
end

local TABS = {
	{ id = "home", label = "Home", icon = "home" },
	{ id = "agents", label = "Agents", icon = "bot" },
	{ id = "music", label = "Now playing", icon = "music-note" },
	{ id = "tray", label = "Tray", icon = "tray" },
	{ id = "alerts", label = "Alerts", icon = "bell" },
	{ id = "notes", label = "Note", icon = "note-text" },
	{ id = "timer", label = "Timer", icon = "timer" },
}

-- The tabs: each a glyph, the open one a white pill that also says its
-- name. Alerts and the tray carry a dot while they hold something.
local function tabs(): Node
	-- (Bare glyphs, as the Mac's own controls: the open one white on a
	-- faint square, the rest at half light. No bar behind them.)
	local row: { [any]: any } = {
		gap = 2,
		p = 3,
		items = "center",
		rounded = "$radius.full",
		bg = ui.raw("#00000042", "the tabs' tray, sunk into the glass"),
		border = 1,
		border_color = ui.raw("#ffffff14", "the tabs' tray's rim"),
	}
	for _, t in TABS do
		local function on(): boolean
			return tab:get() == t.id
		end
		local function dot(): boolean
			if on() then
				return false
			end
			if t.id == "alerts" then
				return #alerts:get() > 0
			elseif t.id == "tray" then
				return #tray:get() > 0
			elseif t.id == "agents" then
				return #attention:get() > 0 or #working:get() > 0
			elseif t.id == "timer" then
				return timer_on:get()
			elseif t.id == "music" then
				local m = media:get()
				return m ~= nil and m.playing
			end
			return false
		end
		local wide = ui.computed(function(): number
			return if on() then 46 + #t.label * 7 else 34
		end)
		table.insert(
			row,
			ui.button {
				w = ui.spring(wide, SPRING),
				h = 30,
				gap = 6,
				shrink = 0,
				overflow = "hidden",
				items = "center",
				justify = "center",
				rounded = "$radius.full",
				relative = true,
				bg = function()
					return if on() then ui.raw("#ffffff2e", "the open tab's lit pill") else "#00000000"
				end,
				hover = { bg = "#ffffff12" },
				active = { scale = 0.94 },
				color = function()
					return if on() then "#ffffff" else "#ffffff73"
				end,
				label = t.label,
				tooltip = t.label,
				on_click = function()
					tab:set(t.id)
				end,
				kx.Icon { name = t.icon, size = 15 },
				ui.show(on, ui.text { t.label, literal = true, shrink = 0, text_size = ui.raw(12, "the open tab's name"), weight = "$font.weight.semibold", color = "#ffffff" }),
				ui.show(dot, ui.div {
					absolute = true,
					right = 5,
					top = 5,
					w = 5,
					h = 5,
					rounded = "$radius.full",
					bg = function(): any
						if t.id == "agents" then
							return if #attention:get() > 0 then "$kx.attention" else ui.raw("#8fb4ff", "agents at work")
						elseif t.id == "timer" then
							return ui.raw("#ffb454", "a timer running")
						elseif t.id == "music" then
							return ui.raw(tint(), "the cover's light")
						end
						return "$accent"
					end,
				}),
			}
		)
	end
	return ui.row(row)
end

local function agents_page(): Node
	return ui.show(function()
		return #all_threads:get() > 0
	end, ui.column {
		w_full = true,
		h_full = true,
		gap = 6,
		section("Agents", function()
			local busy, waits = #working:get(), #attention:get()
			local parts = {}
			if busy > 0 then
				table.insert(parts, `{busy} working`)
			end
			if waits > 0 then
				table.insert(parts, `{waits} waiting`)
			end
			return if #parts == 0 then tostring(#all_threads:get()) else table.concat(parts, " · ")
		end, pill("New", true, function()
			if not fixture then
				pcall(ui.invoke, "notch.ask")
			end
		end)),
		tile {
			w_full = true,
			grow = 1,
			min_h = 0,
			p = 4,
			gap = 0,
			ui.scroll {
				w_full = true,
				h_full = true,
				ui.column {
					w_full = true,
					gap = 0,
					ui.each(all_threads, agent_line, {
						key = function(t: store.Thread)
							return `{t.id}:{t.status}`
						end,
					}),
				},
			},
		},
	}, empty_state("bot", "No agents yet", "Ask one from Home, or press Alt Space from anywhere.", pill("Go to Home", false, function()
		tab:set("home")
	end)))
end

local function page(id: string): Node
	if id == "agents" then
		return agents_page()
	elseif id == "music" then
		return music_row(true)
	elseif id == "notes" then
		return notes_page()
	elseif id == "timer" then
		return timer()
	elseif id == "tray" then
		return tray_page()
	elseif id == "alerts" then
		return alerts_page()
	end
	return home()
end

local function expanded_header(): Node
	return ui.row {
		w_full = true,
		h = 38,
		shrink = 0,
		justify = "between",
		items = "center",
		tabs(),
		round_button("arrow-up-right", "Open Keplorayx", function()
			open_thread(nil)
		end),
	}
end

local function expanded_view(): Node
	return ui.presence(function(): string
		return if chat:get() then "chat" else "tabs"
	end, function(which: string): Node
		if which == "chat" then
			return ui.column { w_full = true, h_full = true, px = 14, pt = 12, pb = 14, chat_view() }
		end
		return ui.column {
			w_full = true,
			h_full = true,
			px = 14,
			pt = 12,
			pb = 14,
			gap = 12,
			expanded_header(),
			ui.column {
				w_full = true,
				grow = 1,
				min_h = 0,
				ui.presence(tab, page, {
					w_full = true,
					h_full = true,
					enter = { opacity = { 0, 1 }, y = { 10, 0 }, scale = { 0.98, 1 }, duration = 280, ease = "outCubic" },
					exit = { opacity = 0, duration = 70 },
					mode = "wait",
				}),
			},
		}
	end, {
		w_full = true,
		h_full = true,
		enter = { opacity = { 0, 1 }, x = { 10, 0 }, duration = 220 },
		exit = { opacity = 0, duration = 70 },
		mode = "wait",
	})
end

-- ── The island ─────────────────────────────────────────────────────────

local function shape_of(kind: string, page: string, slim_as: string, talking: boolean, rows: number, playing: boolean): Shape
	if kind == "hidden" then
		-- While something waits, a warm sliver at the edge; else the pill
		-- waits above it, out of sight, to drop in from there.
		return if first:peek()
			then { w = 140, h = 5, r = 2.5, y = 0, interactive = false }
			else { w = 180, h = 32, r = 16, y = -44, interactive = false }
	elseif kind == "slim" then
		return if slim_as == "waiting"
			then { w = 430, h = 44, r = 22, y = FLOAT, interactive = true }
			elseif slim_as == "resting" then { w = 210, h = 34, r = 17, y = FLOAT, interactive = true }
			elseif slim_count(slim_as) >= 3 then { w = 430, h = 38, r = 19, y = FLOAT, interactive = true }
			elseif slim_count(slim_as) == 2 then { w = 370, h = 36, r = 18, y = FLOAT, interactive = true }
			else { w = 300, h = 34, r = 17, y = FLOAT, interactive = true }
	elseif kind == "alert" then
		return { w = 440, h = 64, r = 28, y = FLOAT, interactive = true }
	elseif kind == "flash" then
		return { w = 340, h = 40, r = 20, y = FLOAT, interactive = true }
	end
	-- Opened, every page is the same width, so moving between them only
	-- grows or shrinks the island; each is as tall as what it holds. (The
	-- frame round a page: its padding, the tabs and the gap under them.)
	local W, R, CHROME = 460, 38, 76
	if talking then
		return { w = 480, h = 400, r = R, y = FLOAT, interactive = true }
	end
	local h = 300
	if page == "home" then
		h = CHROME + 48 + 10 + (if playing then 74 else 0) + (if timer_on:get() then 64 else 0) + 12 + 28 + math.max(1, rows) * 46 + 10 + 44
	elseif page == "music" then
		h = 284
	elseif page == "agents" then
		local n = #all_threads:get()
		h = if n == 0 then 290 else CHROME + 34 + 14 + math.min(5, n) * 46
	elseif page == "tray" then
		local n = #tray:get()
		local lines = math.clamp(math.ceil(n / 4), 1, 2)
		h = if n == 0 then 260 else CHROME + 36 + lines * 140
	elseif page == "alerts" then
		local n = #alerts:get()
		h = if n == 0 then 270 else CHROME + 34 + 14 + math.min(5, n) * 46
	elseif page == "notes" then
		h = 300
	elseif page == "timer" then
		h = 320
	end
	return { w = W, h = math.min(h, 400), r = R, y = FLOAT, interactive = true }
end

local aim = ui.computed(function(): Shape
	return shape_of(state:get(), tab:get(), slim_kind:get(), chat:get() ~= nil, math.min(3, #home_threads:get()), media:get() ~= nil)
end)
ui.effect(function()
	local a = aim:get()
	pcall(ui.invoke, "notch.size", { w = a.w, h = a.h, r = a.r, y = a.y, interactive = a.interactive })
end)
local aim_w = ui.computed(function(): number
	return aim:get().w
end)
local aim_h = ui.computed(function(): number
	return aim:get().h
end)
local aim_r = ui.computed(function(): number
	return aim:get().r
end)
local aim_y = ui.computed(function(): number
	return aim:get().y
end)

-- Its glass's tint, over the blurred window behind it (src/notch.rs
-- `behind`). Compact shapes stay near black. Opened, the top is smoked
-- so the words read, and the foot thins so that window shows through:
-- clearer over a bright one, a little more smoke over a dark one.
local SMOKE = { angle = 180, stops = { { 0, "#000000f2" }, { 1, "#000000e6" } } }
local behind_light: State<number> = ui.shared("notch.behind.light", 0.5)
local function foot(clear: boolean): string
	local l = math.clamp(behind_light:get(), 0, 1)
	local base = if clear then 0.1 else 0.34
	local a = math.floor(math.clamp(base + (1 - l) * 0.42, 0.08, 0.82) * 255)
	return ui.raw(string.format("#000000%02x", a), "smoke over the window behind, thinner when that window is bright")
end
-- Opened: one even smoke, dark enough that the words always read and the
-- window behind only colours it (the Mac's island is black; ours lets a
-- little through). Playing music, the cover's colour, faintly.
-- Liquid glass, as Apple's: the window behind shows through, blurred and
-- its colours deepened, under a tint just dark enough that the words read,
-- darker over a bright window than a dark one. `base` is how much smoke
-- over a dark window; a bright one adds up to 0.3 more.
local function smoke(tone: string, base: number, foot_less: number): any
	local l = math.clamp(behind_light:get(), 0, 1)
	local function a(x: number): string
		return string.format("%02x", math.floor(math.clamp(x, 0, 1) * 255))
	end
	local top = base + l * 0.3
	return {
		angle = 180,
		stops = {
			{ 0, ui.raw(tone .. a(top), "the island's glass tint") },
			{ 1, ui.raw(tone .. a(top - foot_less), "the island's glass tint, its foot") },
		},
	}
end
local function glass_of(clear: boolean): any
	local _ = foot
	local tone = if clear then mix("#0b0b0f", tint(), 0.22) else "#0b0b0f"
	-- (Thin: the window behind shows through, frosted; its words still read.)
	return smoke(tone, 0.27, 0.07)
end
local function colour(): any
	if state:get() ~= "expanded" then
		-- Compact, a little more smoke: a word or two on a small pill.
		local _ = SMOKE
		return smoke("#08080b", 0.4, 0.04)
	end
	return glass_of(tab:get() == "music")
end
-- The glass's light: a bright lip along its top edge fading down, and a
-- faint one along its foot, as light catches a lens's rim.
local SHEEN = {
	angle = 180,
	stops = {
		{ 0, ui.raw("#ffffff3d", "the glass's lit top edge") },
		{ 0.22, ui.raw("#ffffff0a", "the glass's lit top edge, fading") },
		{ 0.5, ui.raw("#ffffff00", "the glass's middle, unlit") },
		{ 0.86, ui.raw("#ffffff00", "the glass's foot, unlit") },
		{ 1, ui.raw("#ffffff14", "the glass's foot, catching light") },
	},
}

-- A lens's sides: light gathers at its left and right edges, as it does
-- round a drop of glass, and the middle stays clear.
local EDGES = {
	angle = 90,
	stops = {
		{ 0, ui.raw("#ffffff1a", "the glass's lit left edge") },
		{ 0.1, ui.raw("#ffffff00", "the glass's left edge, fading") },
		{ 0.9, ui.raw("#ffffff00", "the glass's right edge, fading") },
		{ 1, ui.raw("#ffffff1a", "the glass's lit right edge") },
	},
}
-- The specular streak: the brightest light on the glass, along the middle
-- of its top edge, fading out before the corners.
local STREAK = {
	angle = 90,
	stops = {
		{ 0, ui.raw("#ffffff00", "the streak's end") },
		{ 0.3, ui.raw("#ffffff73", "the streak, rising") },
		{ 0.5, ui.raw("#ffffffb3", "the streak at its brightest") },
		{ 0.7, ui.raw("#ffffff73", "the streak, falling") },
		{ 1, ui.raw("#ffffff00", "the streak's end") },
	},
}

-- The light in the glass: two soft lights falling in from its top corners,
-- in the colour of what's going on (warm while something waits, blue while
-- agents work, the cover's own while music plays, a cool dusk at rest).
-- Playing, the two sway against each other in time with the waveform.
local function aurora(left: boolean): any
	local a, b = "#6f8cff", "#b56cff"
	local strength = 0.2
	local m = media:get()
	if first:get() then
		a, b, strength = "#ffb454", "#ff7a59", 0.3
	elseif #working:get() > 0 then
		a, b, strength = "#5aa2ff", "#5ce1e6", 0.26
	elseif timer_on:get() and not (m and m.playing) then
		a, b, strength = "#ffb454", "#ffd38a", 0.22
	elseif m and m.playing then
		a, b = mix(tint(), "#ffffff", 0.12), mix(tint(), "#ffffff", 0.4)
		local sway = math.sin(wave:get() * 1.3 + (if left then 0 else math.pi))
		strength = 0.26 + 0.1 * sway
	end
	if state:get() ~= "expanded" then
		strength *= 0.8
	end
	local hue = if left then a else b
	local top = string.format("%02x", math.floor(math.clamp(strength, 0, 1) * 255))
	return {
		angle = if left then 125 else 235,
		stops = {
			{ 0, ui.raw(hue .. top, "the light in the glass") },
			{ 0.5, ui.raw(hue .. "00", "the light in the glass, spent") },
			{ 1, ui.raw(hue .. "00", "the light in the glass, spent") },
		},
	}
end

-- What's behind, pinned to the screen while the pill springs: its place in
-- the pill is the pill's own place negated, on the same spring (a spring is
-- linear, so the two stay in register).
local behind_copies: State<number> = ui.shared("notch.behind", 0)
local behind_left = ui.computed(function(): number
	return (aim_w:get() - 680) / 2 - 1
end)
local behind_top = ui.computed(function(): number
	return -aim_y:get() - 1
end)

-- The waveform moves while it shows and plays (nothing ticks otherwise).
local wave_stop: (() -> ())? = nil
ui.effect(function()
	local m = media:get()
	local s = state:get()
	local showing = (s == "slim" and string.find(slim_kind:get(), "music", 1, true) ~= nil) or (s == "expanded" and (tab:get() == "music" or tab:get() == "home"))
	if m and m.playing and showing then
		if not wave_stop then
			wave_stop = ui.every(110, function()
				wave:set(wave:peek() + 0.11)
			end)
		end
	elseif wave_stop then
		wave_stop()
		wave_stop = nil
	end
end)

local function view(kind: string): Node
	if kind == "hidden" then
		return ui.div { w_full = true, h_full = true, bg = "$kx.attention", opacity = glow }
	elseif kind == "slim" then
		return ui.button {
			w_full = true,
			h_full = true,
			bg = "#00000000",
			on_click = function()
				expanded:set(true)
			end,
			label = "Keplorayx: open the island",
			slim(),
		}
	elseif kind == "alert" then
		return ui.row {
			w_full = true,
			h_full = true,
			px = 22,
			items = "center",
			asking(function()
				return first:get()
			end, false),
		}
	elseif kind == "flash" then
		return ui.button {
			w_full = true,
			h_full = true,
			px = 18,
			bg = "#00000000",
			items = "center",
			on_click = function()
				local f = flash:peek()
				open_thread(if f and f.id ~= "timer" then f.id else nil)
			end,
			label = "Open the thread",
			flash_view(),
		}
	end
	return expanded_view()
end

-- The pill: black where it meets the screen's edge, melting into the glass
-- below (the window's Acrylic shows through), a hairline of light round it,
-- warm while something waits. Its size and corners spring as the glass
-- does; what's in it keeps its final size, so the pill reveals it rather
-- than squeezing it, and it fades and sharpens in as the shape arrives.
local island = ui.column {
	id = "island",
	theme = "zeron-dark",
	w = ui.spring(aim_w, SPRING),
	h = ui.spring(aim_h, SPRING),
	y = ui.spring(aim_y, SPRING),
	rounded = ui.spring(aim_r, SPRING),
	overflow = "hidden",
	items = "center",
	-- A soft shadow that fades out well inside the window's cut (14 points
	-- round the pill, 21 under it, src/kx/notch.rs `SHADOW`): one reaching
	-- past it was sliced off there, a hard-edged box seen round the island.
	shadow = { x = 0, y = 5, blur = 13, spread = -3, color = ui.raw("#00000066", "the island's soft shadow") },
	-- Its rim: a hairline of light, brighter along the top (the inset lip).
	border = 1,
	border_color = function()
		if first:get() then
			return "$kx.attention.soft"
		end
		if #working:get() > 0 then
			return ui.raw("#9cc2ff66", "the glass's rim while agents work")
		elseif timer_on:get() then
			return ui.raw("#ffc77352", "the glass's rim while a timer runs")
		end
		return ui.raw("#ffffff3d", "the glass's rim")
	end,
	inset_shadow = { x = 0, y = 1, blur = 0, spread = 0, color = ui.raw("#ffffff3d", "the glass's lit top lip") },
	-- The glass: what's behind, blurred, its colour deepened.
	ui.image {
		src = function()
			behind_copies:get()
			return "host:notch/behind"
		end,
		absolute = true,
		left = ui.spring(behind_left, SPRING),
		top = ui.spring(behind_top, SPRING),
		w = 680,
		h = 460,
		fit = "fill",
		-- (Already frosted: blurred, deepened and lifted in src/kx/notch.rs.
		-- A blur here would draw from a layer of its own that the pill's
		-- rounded clip doesn't hold, the whole copy seen round it as a box.)
	},
	-- Its tint, and the light on it.
	ui.div { absolute = true, left = 0, top = 0, right = 0, bottom = 0, gradient = colour },
	ui.div { absolute = true, left = 0, top = 0, right = 0, bottom = 0, pointer_events = "none", gradient = function()
		return aurora(true)
	end },
	ui.div { absolute = true, left = 0, top = 0, right = 0, bottom = 0, pointer_events = "none", gradient = function()
		return aurora(false)
	end },
	ui.div { absolute = true, left = 0, top = 0, right = 0, bottom = 0, pointer_events = "none", gradient = SHEEN },
	ui.div { absolute = true, left = 0, top = 0, right = 0, bottom = 0, pointer_events = "none", gradient = EDGES },
	ui.div {
		absolute = true,
		top = 0,
		left = function()
			return math.floor(aim_w:get() * 0.18)
		end,
		right = function()
			return math.floor(aim_w:get() * 0.18)
		end,
		h = 1,
		pointer_events = "none",
		gradient = STREAK,
	},
	-- (Inside the hairline: its final size less the border's two points.)
	ui.column {
		w = function()
			return aim_w:get() - 2
		end,
		h = function()
			return aim_h:get() - 2
		end,
		shrink = 0,
		-- A morph, not a swap: the shape leads, and what it holds follows a
		-- beat behind, coming into focus as it grows to full size, so the
		-- pill reads as becoming the panel. Leaving, it just goes soft.
		ui.presence(state, view, {
			w_full = true,
			h_full = true,
			-- (No blur here: a blurred layer is drawn as its own box, past the
			-- pill's rounded clip, and showed as a window round the island.)
			enter = { opacity = { 0, 1 }, scale = { 0.92, 1 }, y = { 6, 0 }, duration = 340, delay = 50, ease = "outCubic" },
			exit = { opacity = { 1, 0 }, scale = { 1, 0.98 }, duration = 80, ease = kx.motion.EXIT },
			mode = "wait",
		}),
	},
}

return ui.row {
	w_full = true,
	h_full = true,
	justify = "center",
	items = "start",
	on_file_drop = function(files: { any })
		keep(files)
		tab:set("tray")
		expanded:set(true)
	end,
	island,
}
