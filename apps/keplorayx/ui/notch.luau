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

-- Each state's pill, in points: its width and height, its corners, how far
-- below the screen's edge it floats (above it, out of sight, while it
-- rests), and whether it takes clicks. (The Dynamic Island's: compact, a
-- banner, and expanded by what it shows.)
type Shape = { w: number, h: number, r: number, y: number, interactive: boolean }
-- (The gap between the screen's edge and the island.)
local FLOAT = 8
local SPRING = { stiffness = 340, damping = 26 }

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

-- A key on the island: its glyph alone, white, dimming under the pointer.
local function key(icon: string, what: string, name: string, size: number?): Node
	local n = size or 16
	return ui.button {
		w = n + 18,
		h = n + 18,
		rounded = "$radius.full",
		items = "center",
		justify = "center",
		bg = "#00000000",
		color = "#ffffffeb",
		hover = { bg = "#ffffff14" },
		active = { scale = 0.9 },
		label = name,
		on_click = function()
			control(what)
		end,
		kx.Icon { name = icon, size = n },
	}
end

local function cover(size: number, corner: number?): Node
	local round = corner or math.floor(size * 0.22)
	return ui.presence(function(): string
		local m = media:get()
		return if m and m.cover ~= "" then m.cover else ""
	end, function(src: string): Node
		if src == "" then
			return ui.column {
				w = size,
				h = size,
				rounded = round,
				bg = "#ffffff1a",
				items = "center",
				justify = "center",
				color = "#ffffffb3",
				shrink = 0,
				kx.Icon { name = "music-note", size = math.floor(size * 0.42) },
			}
		end
		return ui.image { src = src, w = size, h = size, rounded = round, fit = "cover", shrink = 0 }
	end, { enter = { opacity = { 0, 1 }, scale = { 0.92, 1 }, duration = 260 } })
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

-- The waveform, in the cover's light (src/media.rs `tint`), moving while
-- it plays and resting low while it doesn't: the Dynamic Island's.
local wave = ui.state("notch.wave", 0)
local WAVE = { 0.55, 0.9, 0.7, 1, 0.6, 0.85 }
local function tint(): string
	local m = media:get()
	return if m and (m :: any).tint and (m :: any).tint ~= "" then (m :: any).tint else "#c9b8ff"
end
local function waveform(bars: number, height: number, width: number?): Node
	local row: { [any]: any } = { gap = 3, items = "center", h = height, shrink = 0 }
	for i = 1, bars do
		local base = WAVE[(i - 1) % #WAVE + 1]
		table.insert(row, ui.div {
			w = width or 3,
			rounded = "$radius.full",
			h = function()
				local m = media:get()
				if not (m and m.playing) then
					return math.max(3, height * 0.18)
				end
				local t = wave:get()
				local swing = math.abs(math.sin(t * (0.9 + i * 0.37) + i * 1.7))
				return math.max(3, height * base * (0.3 + 0.7 * swing))
			end,
			gradient = function()
				local c = tint()
				return { angle = 180, stops = { { 0, c }, { 1, c .. "8c" } } }
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
		px = 14,
		gap = "$space.3",
		items = "center",
		lead,
		ui.row { grow = 1, min_w = 0, justify = "center", items = "center", middle },
		trail,
	}
end

local function white(text: any, size: string?, weight: string?): Node
	return ui.text { text, literal = true, truncate = true, color = "#ffffffeb", text_size = size or "$font.size.sm", weight = weight or "$font.weight.medium" }
end

local slim_resting = compact(
	ui.row {
		gap = "$space.2",
		items = "center",
		ui.column { color = "$accent", kx.Icon { name = "orbit", size = 14 } },
		white(clock, nil, "$font.weight.semibold"),
	},
	quiet(day, "$font.size.xs"),
	keys("Alt", "Space")
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

local slim_kind = ui.computed(function(): string
	if first:get() then
		return "waiting"
	end
	if #working:get() > 0 then
		return "working"
	end
	local m = media:get()
	return if m and m.playing then "music" else "resting"
end)

local slim_music = compact(cover(22, 6), white(song("title"), "$font.size.xs"), waveform(4, 14))

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
		return if kind == "working" then slim_working elseif kind == "music" then slim_music else slim_resting
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
	return ui.row {
		gap = "$space.3",
		items = "center",
		w_full = true,
		ui.text { function()
			return mmss(position())
		end, literal = true, text_size = "$font.size.xs", color = "#ffffff99", tabular = true, w = 38 },
		ui.div {
			h = 6,
			grow = 1,
			rounded = "$radius.full",
			bg = "#ffffff2e",
			overflow = "hidden",
			on_click = function(e: any)
				local m = media:peek()
				if m and m.duration > 0 and e and e.local_x and e.width then
					control({ seek = e.local_x / e.width })
				end
			end,
			ui.div {
				h_full = true,
				rounded = "$radius.full",
				bg = "#ffffffc7",
				w = function()
					return `{math.floor(fraction() * 1000) / 10}%`
				end,
			},
		},
		ui.text { function()
			local m = media:get()
			return if m and m.duration > 0 then `-{mmss(m.duration - position())}` else ""
		end, literal = true, text_size = "$font.size.xs", color = "#ffffff99", tabular = true, w = 44, align = "right" },
	}
end

local function play_key(size: number): Node
	return ui.presence(function(): string
		local m = media:get()
		return if m and m.playing then "pause" else "play"
	end, function(k: string): Node
		return key(if k == "pause" then "media-pause" else "media-play", "play", if k == "pause" then "Pause" else "Play", size)
	end, { enter = { opacity = { 0, 1 }, scale = { 0.7, 1 }, duration = 180 }, exit = { opacity = 0, duration = 60 }, mode = "wait" })
end

local function music_row(big: boolean): Node
	if not big then
		return ui.row {
			gap = "$space.3",
			items = "center",
			w_full = true,
			p = "$space.2",
			rounded = 18,
			bg = "#ffffff0d",
			cover(40, 10),
			ui.column {
				grow = 1,
				min_w = 0,
				white(song("title"), "$font.size.sm", "$font.weight.semibold"),
				quiet(song("by"), "$font.size.xs"),
			},
			waveform(5, 18),
			key("media-previous", "previous", "Previous", 14),
			play_key(16),
			key("media-next", "next", "Next", 14),
		}
	end
	return ui.column {
		w_full = true,
		gap = "$space.4",
		ui.row {
			gap = "$space.4",
			items = "center",
			w_full = true,
			cover(64, 14),
			ui.column {
				grow = 1,
				min_w = 0,
				gap = 2,
				white(song("title"), "$font.size.md", "$font.weight.bold"),
				ui.text { song("by"), literal = true, truncate = true, text_size = "$font.size.sm", color = "#ffffff8c" },
			},
			waveform(6, 30, 4),
		},
		progress(),
		ui.row {
			gap = 36,
			items = "center",
			justify = "center",
			w_full = true,
			key("media-rewind", "previous", "Previous", 32),
			play_key(38),
			key("media-forward", "next", "Next", 32),
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

-- A tile on the island: a rounded well of light on the black.
local function tile(props: { [any]: any }): Node
	local p: { [any]: any } = {
		rounded = 22,
		bg = "#ffffff12",
		border = 1,
		border_color = "#ffffff1a",
		inset_shadow = { x = 0, y = 1, blur = 0, spread = 0, color = "#ffffff12" },
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
		text_size = ui.raw(10, "a tile's label"),
		weight = "$font.weight.semibold",
		color = "#ffffff73",
		letter_spacing = 0.6,
	}
end

local function grey(text: any, size: string?): Node
	return ui.text { text, literal = true, truncate = true, text_size = size or "$font.size.xs", color = "#ffffff8c" }
end

-- An agent at a glance: its mark, its thread, how it stands.
local function agent_line(t: store.Thread): Node
	local waits: Waiting? = nil
	for _, w in attention:peek() do
		if w.thread_id == t.id then
			waits = w
		end
	end
	return ui.button {
		w_full = true,
		h = 34,
		gap = "$space.2",
		items = "center",
		px = 6,
		rounded = 12,
		bg = "#00000000",
		hover = { bg = "#ffffff0f" },
		on_click = function()
			open_thread(t.id)
		end,
		kx.AgentMark { agent = t.harness_id, size = 16 },
		ui.text { t.title, literal = true, truncate = true, grow = 1, min_w = 0, text_size = "$font.size.sm", color = "#ffffffe6" },
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
			elseif t.status == "failed" then kx.StatusIcon { kind = "failed", size = 13 }
			else kx.StatusIcon { kind = "done", size = 13 },
	}
end

-- The ask: a pill to type in, and a round key to send.
local function ask_pill(): Node
	return ui.row {
		w_full = true,
		h = 40,
		gap = "$space.2",
		items = "center",
		pl = 14,
		pr = 4,
		rounded = "$radius.full",
		bg = "#ffffff12",
		border = 1,
		border_color = "#ffffff0d",
		ui.column { color = "$accent", kx.Icon { name = "orbit", size = 15 } },
		ui.input {
			value = ask_text,
			placeholder = "Ask an agent…",
			placeholder_color = "#ffffff66",
			label = "Ask an agent",
			grow = 1,
			min_w = 0,
			h = 32,
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
			w = 32,
			h = 32,
			rounded = "$radius.full",
			items = "center",
			justify = "center",
			bg = function()
				return if string.match(ask_text:get(), "^%s*$") then "#ffffff26" else "#ffffff"
			end,
			color = "#000000",
			label = "Ask",
			on_click = ask,
			ui.show(ask_busy, kx.Spinner { size = 12 }, kx.Icon { name = "arrow-up", size = 15 }),
		},
	}
end

local function home(): Node
	return ui.column {
		gap = 10,
		w_full = true,
		ui.row {
			gap = 10,
			w_full = true,
			h = 132,
			-- The time.
			tile {
				w = 196,
				h_full = true,
				justify = "between",
				label(function()
					return now:get().weekday
				end),
				ui.text { clock, literal = true, font = "$kx.display", text_size = ui.raw(52, "the island's clock"), color = "#ffffff", tabular = true },
				grey(function()
					local t = now:get()
					return `{t.day} {t.month}`
				end, "$font.size.sm"),
			},
			-- What's playing.
			tile {
				grow = 1,
				min_w = 0,
				h_full = true,
				justify = "between",
				ui.row {
					w_full = true,
					gap = "$space.3",
					items = "center",
					cover(48, 12),
					ui.column {
						grow = 1,
						min_w = 0,
						gap = 1,
						white(song("title"), "$font.size.sm", "$font.weight.semibold"),
						grey(song("by")),
					},
					waveform(5, 20),
				},
				ui.row {
					w_full = true,
					justify = "center",
					gap = 22,
					items = "center",
					key("media-rewind", "previous", "Previous", 18),
					play_key(22),
					key("media-forward", "next", "Next", 18),
				},
			},
		},
		-- Your agents.
		tile {
			w_full = true,
			gap = 4,
			py = 10,
			ui.row {
				w_full = true,
				justify = "between",
				items = "center",
				px = 6,
				label("Agents"),
				grey(function()
					local n = #working:get()
					return if n == 0 then "None working" elseif n == 1 then "1 working" else `{n} working`
				end),
			},
			ui.show(function()
				return #home_threads:get() > 0
			end, ui.each(home_threads, agent_line, {
				key = function(t: store.Thread)
					return `{t.id}:{t.status}`
				end,
				gap = 0,
			}), ui.row {
				w_full = true,
				h = 34,
				px = 6,
				items = "center",
				grey("Nothing yet. Ask one below.", "$font.size.sm"),
			}),
		},
		ask_pill(),
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
		table.insert(presets, pill(`{m} min`, false, function()
			start_timer(m)
		end))
	end
	return ui.column {
		gap = "$space.4",
		w_full = true,
		h_full = true,
		items = "center",
		justify = "center",
		ui.text {
			function()
				local l = timer_left:get()
				return string.format("%d:%02d", math.floor(l / 60), l % 60)
			end,
			literal = true,
			font = "$kx.display",
			text_size = ui.raw(72, "the timer's figures"),
			color = function()
				return if timer_left:get() > 0 then "#ffffff" else "#ffffff59"
			end,
			tabular = true,
		},
		ui.presence(function(): string
			return if timer_left:get() > 0 then "running" else "idle"
		end, function(k: string): Node
			if k == "running" then
				return pill("Stop", true, function()
					timer_end:set(0)
					timer_left:set(0)
				end)
			end
			return ui.row(presets)
		end, { enter = { opacity = { 0, 1 }, duration = 140 }, exit = { opacity = 0, duration = 60 }, mode = "wait" }),
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

local function tray_page(): Node
	local function file_tile(k: Kept): Node
		local ext = string.upper(string.match(k.name, "%.([%w]+)$") or "FILE")
		return ui.column {
			w = 92,
			gap = 6,
			items = "center",
			ui.button {
				w = 72,
				h = 72,
				rounded = 18,
				bg = "#ffffff12",
				border = 1,
				border_color = "#ffffff0d",
				hover = { bg = "#ffffff1f" },
				items = "center",
				justify = "center",
				gap = 2,
				label = `Open {k.name}`,
				tooltip = k.path,
				on_click = function()
					pcall(ui.invoke, "open_url", k.path)
				end,
				ui.column { color = "#ffffffcc", kx.Icon { name = "document", size = 24 } },
				ui.text { string.sub(ext, 1, 5), literal = true, text_size = ui.raw(9, "a file's kind"), weight = "$font.weight.semibold", color = "#ffffff80" },
			},
			ui.text { k.name, literal = true, truncate = true, max_w = 92, text_size = "$font.size.xs", color = "#ffffffd9" },
			ui.row {
				gap = 2,
				ui.button { w = 22, h = 22, rounded = "$radius.full", items = "center", justify = "center", bg = "#00000000", hover = { bg = "#ffffff1a" }, color = "#ffffff8c", label = "Show in folder", tooltip = "Show in folder", on_click = function()
					pcall(ui.invoke, "notch.reveal", k.path)
				end, kx.Icon { name = "folder", size = 12 } },
				ui.button { w = 22, h = 22, rounded = "$radius.full", items = "center", justify = "center", bg = "#00000000", hover = { bg = "#ffffff1a" }, color = "#ffffff8c", label = "Take it away", tooltip = "Take it away", on_click = function()
					local list = {}
					for _, x in tray:peek() do
						if x.path ~= k.path then
							table.insert(list, x)
						end
					end
					tray:set(list)
				end, kx.Icon { name = "close", size = 11 } },
			},
		}
	end
	return ui.show(
		function()
			return #tray:get() > 0
		end,
		ui.scroll {
			w_full = true,
			h_full = true,
			ui.row {
				w_full = true,
				wrap = true,
				gap = 8,
				ui.each(tray, file_tile, {
					key = function(k: Kept)
						return k.path
					end,
				}),
			},
		},
		ui.column {
			w_full = true,
			h_full = true,
			items = "center",
			justify = "center",
			gap = "$space.2",
			rounded = 22,
			border = 1,
			border_color = "#ffffff24",
			border_style = "dashed",
			ui.column { color = "#ffffff99", kx.Icon { name = "tray", size = 28 } },
			white("Drop files here", "$font.size.sm", "$font.weight.semibold"),
			grey("Drag them to the top of the screen. They wait here until you take them away."),
		}
	)
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
			ui.row {
				w_full = true,
				justify = "between",
				items = "center",
				px = 6,
				label("Recent"),
				pill("Clear", false, function()
					alerts:set({})
				end),
			},
			ui.scroll {
				w_full = true,
				grow = 1,
				ui.column {
					w_full = true,
					gap = 0,
					ui.each(alerts, function(a: Alert): Node
						return ui.button {
							w_full = true,
							h = 38,
							gap = "$space.3",
							items = "center",
							px = 8,
							rounded = 12,
							bg = "#00000000",
							hover = { bg = "#ffffff0f" },
							on_click = function()
								open_thread(a.thread)
							end,
							kx.StatusIcon { kind = if a.kind == "failed" then "failed" elseif a.kind == "asked" then "approval" else "done", size = 14 },
							ui.text { a.title, literal = true, truncate = true, grow = 1, min_w = 0, text_size = "$font.size.sm", color = "#ffffffe6" },
							ui.text { ago(a.at), literal = true, text_size = "$font.size.xs", color = "#ffffff73", tabular = true },
						}
					end, {
						key = function(a: Alert)
							return `{a.thread}:{a.at}:{a.kind}`
						end,
					}),
				},
			},
		},
		ui.column {
			w_full = true,
			h_full = true,
			items = "center",
			justify = "center",
			gap = "$space.2",
			ui.column { color = "#ffffff80", kx.Icon { name = "bell", size = 28 } },
			white("All quiet", "$font.size.sm", "$font.weight.semibold"),
			grey("Finishes, failures and what agents ask land here."),
		}
	)
end

local TABS = {
	{ id = "home", label = "Home", icon = "home" },
	{ id = "agents", label = "Agents", icon = "bot" },
	{ id = "music", label = "Music", icon = "music-note" },
	{ id = "tray", label = "Tray", icon = "tray" },
	{ id = "alerts", label = "Alerts", icon = "bell" },
	{ id = "notes", label = "Notes", icon = "note-text" },
	{ id = "timer", label = "Timer", icon = "timer" },
}

local function tabs(): Node
	local row: { [any]: any } = { gap = 2, items = "center", p = 3, rounded = "$radius.full", bg = "#ffffff12" }
	for _, t in TABS do
		table.insert(
			row,
			ui.button {
				w = 28,
				h = 28,
				items = "center",
				justify = "center",
				rounded = "$radius.full",
				bg = function()
					return if tab:get() == t.id then "#ffffff2e" else "#00000000"
				end,
				hover = { bg = "#ffffff1a" },
				color = function()
					return if tab:get() == t.id then "#ffffff" else "#ffffff80"
				end,
				label = t.label,
				tooltip = t.label,
				on_click = function()
					tab:set(t.id)
				end,
				kx.Icon { name = t.icon, size = 14 },
			}
		)
	end
	return ui.row(row)
end

local function page(id: string): Node
	if id == "agents" then
		return ui.show(function()
			return #all_threads:get() > 0
		end, ui.scroll {
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
		}, ui.column {
			w_full = true,
			h_full = true,
			items = "center",
			justify = "center",
			gap = "$space.2",
			ui.column { color = "#ffffff80", kx.Icon { name = "bot", size = 28 } },
			white("No agents yet", "$font.size.sm", "$font.weight.semibold"),
			grey("Ask one from Home, or press Alt Space."),
		})
	elseif id == "music" then
		return ui.column { w_full = true, pt = "$space.2", music_row(true) }
	elseif id == "notes" then
		return tile {
			w_full = true,
			h_full = true,
			p = 4,
			ui.textarea {
				value = note,
				placeholder = "A note to keep…",
				w_full = true,
				grow = 1,
				p = 12,
				color = "#ffffffe6",
				text_size = "$font.size.sm",
				bg = "#00000000",
				border = 0,
			},
		}
	elseif id == "timer" then
		return timer()
	elseif id == "tray" then
		return tray_page()
	elseif id == "alerts" then
		return alerts_page()
	end
	return home()
end

local function expanded_view(): Node
	return ui.column {
		w_full = true,
		h_full = true,
		px = 20,
		pt = 14,
		pb = 18,
		gap = "$space.4",
		ui.row {
			w_full = true,
			justify = "between",
			items = "center",
			tabs(),
			ui.button {
				w = 30,
				h = 30,
				rounded = "$radius.full",
				items = "center",
				justify = "center",
				bg = "#ffffff12",
				hover = { bg = "#ffffff24" },
				color = "#ffffffcc",
				label = "Open Keplorayx",
				tooltip = "Open Keplorayx",
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
			ui.presence(tab, page, {
				w_full = true,
				h_full = true,
				enter = { opacity = { 0, 1 }, y = { 6, 0 }, duration = 200 },
				exit = { opacity = 0, duration = 70 },
				mode = "wait",
			}),
		},
	}
end

-- ── The island ─────────────────────────────────────────────────────────

local function shape_of(kind: string, page: string, slim_as: string): Shape
	if kind == "hidden" then
		-- While something waits, a warm sliver at the edge; else the pill
		-- waits above it, out of sight, to drop in from there.
		return if first:peek()
			then { w = 150, h = 5, r = 2.5, y = 0, interactive = false }
			else { w = 220, h = 36, r = 18, y = -48, interactive = false }
	elseif kind == "slim" then
		return if slim_as == "waiting"
			then { w = 500, h = 48, r = 24, y = FLOAT, interactive = true }
			else { w = 360, h = 38, r = 19, y = FLOAT, interactive = true }
	elseif kind == "alert" then
		return { w = 520, h = 72, r = 30, y = FLOAT, interactive = true }
	elseif kind == "flash" then
		return { w = 400, h = 46, r = 23, y = FLOAT, interactive = true }
	end
	local SHAPES: { [string]: { number } } = {
		music = { 470, 272 },
		home = { 580, 414 },
		agents = { 520, 330 },
		tray = { 520, 268 },
		alerts = { 520, 330 },
		notes = { 480, 290 },
		timer = { 440, 270 },
	}
	local size = SHAPES[page] or SHAPES.home
	return { w = size[1], h = size[2], r = 40, y = FLOAT, interactive = true }
end

local aim = ui.computed(function(): Shape
	return shape_of(state:get(), tab:get(), slim_kind:get())
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

-- Its glass's tint, over what's behind it blurred (src/notch.rs
-- `behind`): the compact shapes the darkest, near the Dynamic Island's
-- black; opened, smoked glass, deepest at its top where the tabs sit and
-- clearing toward its foot (the music view clearest, as the reference).
local SMOKE = { angle = 180, stops = { { 0, "#000000c7" }, { 1, "#000000ad" } } }
local CLEAR = {
	angle = 180,
	stops = { { 0, "#000000b3" }, { 0.5, "#0000008a" }, { 1, "#ffffff33" } },
}
local GLASS = {
	angle = 180,
	stops = { { 0, "#000000b3" }, { 0.6, "#00000094" }, { 1, "#0000006b" } },
}
local function colour(): any
	if state:get() ~= "expanded" then
		return SMOKE
	end
	return if tab:get() == "music" then CLEAR else GLASS
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
	local showing = (s == "slim" and slim_kind:get() == "music") or (s == "expanded" and (tab:get() == "music" or tab:get() == "home"))
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
	theme = "orbit-night",
	w = ui.spring(aim_w, SPRING),
	h = ui.spring(aim_h, SPRING),
	y = ui.spring(aim_y, SPRING),
	rounded = ui.spring(aim_r, SPRING),
	overflow = "hidden",
	items = "center",
	-- (Light caught along its top edge, as glass's; a soft shadow under it.)
	inset_shadow = { x = 0, y = 1, blur = 0, spread = 0, color = "#ffffff24" },
	shadow = { x = 0, y = 10, blur = 28, spread = 0, color = "#00000066" },
	border = 1,
	border_color = function()
		return if first:get() then "$kx.attention.soft" else "#ffffff2b"
	end,
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
		blur = 26,
		saturate = 1.7,
		brightness = 0.95,
	},
	-- Its tint, and a sheen across its top left.
	ui.div { absolute = true, left = 0, top = 0, right = 0, bottom = 0, gradient = colour },
	ui.div {
		absolute = true,
		left = 0,
		top = 0,
		right = 0,
		bottom = 0,
		gradient = { angle = 155, stops = { { 0, "#ffffff1c" }, { 0.32, "#ffffff05" }, { 0.6, "#ffffff00" } } },
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
		ui.presence(state, view, {
			w_full = true,
			h_full = true,
			enter = { opacity = { 0, 1 }, scale = { 0.9, 1 }, y = { -6, 0 }, duration = 300, ease = "cubicBezier(0.32, 1.25, 0.5, 1)" },
			exit = { opacity = { 1, 0 }, scale = { 1, 0.96 }, duration = 90, ease = kx.motion.EXIT },
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
