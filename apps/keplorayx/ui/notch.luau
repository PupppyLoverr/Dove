--!strict
-- The notch: an island hanging from the top edge of the screen, over every
-- window. It shows only when you reach for it (the pointer at the top edge
-- over it, src/notch.rs `notch.near`) and slides back up soon after you
-- leave; hidden, clicks pass through. While an agent waits on you, a thin
-- warm line stays at the edge where it would come from.
--
-- Shown, it says what matters most: who waits on you and why, with Allow
-- and Deny; else your agents at work (each a star) and the first one's
-- step; else, at rest, the time, the day, and a click to ask (the
-- launcher). A click on it brings the main window forward on that thread.

ui.theme.strict("warn")
local kx = require("@kx")
kx.setup()

local store = require("@kx/store")

type Waiting = { thread_id: string, title: string, harness: string, harness_id: string, request_id: string, tool: string, summary: string }

local attention: State<{ Waiting }> = if store.fixture ~= ""
	then ui.state("fx.attention", ((store :: any).fixture_data.attention or {}) :: { Waiting })
	else ui.shared("kx.attention", {} :: { Waiting })

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

local function decide(decision: string)
	local w = first:peek()
	if w and store.fixture == "" then
		ui.invoke("kx.decide_for", { thread_id = w.thread_id, request_id = w.request_id, decision = decision })
	end
end

local function focus()
	local w = first:peek()
	local t = working:peek()[1]
	if not w and not t then
		-- At rest: ask.
		pcall(ui.invoke, "notch.ask")
		return
	end
	local id = if w then w.thread_id elseif t then t.id else nil
	ui.invoke("notch.focus", id)
end

local glow = kx.motion.breath(0.55, 1)

-- The time, to the minute (src/clock.rs); a fixture's is still.
type Now = { hour: number, minute: number, weekday: string, day: number, month: string }
local now: State<Now> = if store.fixture ~= ""
	then ui.state("fx.now", { hour = 21, minute = 42, weekday = "Thursday", day = 5, month = "November" } :: Now)
	else ui.shared("kx.now", { hour = 0, minute = 0, weekday = "", day = 0, month = "" } :: Now)
local function clock(): string
	local t = now:get()
	return string.format("%d:%02d", t.hour, t.minute)
end
local function day(): string
	local t = now:get()
	return if t.weekday == "" then "" else `{string.sub(t.weekday, 1, 3)} {t.day} {string.sub(t.month, 1, 3)}`
end

-- Shown: reached for (or always, for a fixture's picture).
local near: State<boolean> = if store.fixture ~= "" then ui.state("fx.near", true) else ui.shared("notch.near", false)

local function star(t: store.Thread): Node
	return kx.AgentMark { agent = t.harness_id, size = 8, live = function()
		return true
	end }
end

local function quiet(text: any): Node
	return ui.text { text, literal = true, text_size = "$font.size.sm", color = "$text.tertiary", truncate = true }
end

-- At rest: the time and the day, and a way to ask.
local resting = ui.row {
	gap = "$space.3",
	items = "center",
	ui.column { color = "$accent", kx.Icon { name = "orbit", size = 15 } },
	ui.text { clock, literal = true, weight = "$font.weight.semibold", tabular = true },
	quiet(day),
	ui.div { w = 1, h = 14, bg = "$border.subtle" },
	quiet("Ask anything"),
	ui.row {
		gap = 3,
		items = "center",
		kx.Kbd { "Alt" },
		kx.Kbd { "Space" },
	},
}

-- At work: each agent a star, how many, and the first one's step.
local at_work = ui.row {
	gap = "$space.3",
	items = "center",
	ui.row {
		gap = 4,
		items = "center",
		ui.each(working, star, {
			key = function(t: store.Thread)
				return t.id
			end,
			gap = 4,
			items = "center",
		}),
	},
	ui.text {
		function()
			local n = #working:get()
			return if n == 1 then "1 agent working" else `{n} agents working`
		end,
		literal = true,
		weight = "$font.weight.medium",
		text_size = "$font.size.sm",
	},
	ui.div { w = 1, h = 14, bg = "$border.subtle" },
	ui.text {
		function()
			local t = working:get()[1]
			return if t then t.title else ""
		end,
		literal = true,
		truncate = true,
		max_w = 220,
		text_size = "$font.size.sm",
		color = "$text.secondary",
	},
}

-- Waiting on you: who and why, and the answer right there.
local waiting = ui.row {
	gap = "$space.3",
	items = "center",
	ui.div { w = 8, h = 8, rounded = "$radius.full", bg = "$kx.attention", opacity = glow },
	ui.column {
		gap = 0,
		max_w = 250,
		ui.text {
			function()
				local w = first:get()
				return if w then `{w.harness} wants to run {w.tool}` else ""
			end,
			literal = true,
			truncate = true,
			text_size = "$font.size.sm",
			weight = "$font.weight.medium",
		},
		ui.text {
			function()
				local w = first:get()
				return if w then w.title else ""
			end,
			literal = true,
			truncate = true,
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		},
	},
	kx.Button { tone = "ghost", size = "xs", shape = "pill", on_click = function()
		decide("deny")
	end, "Deny" },
	kx.Button { tone = "attention", size = "xs", shape = "pill", on_click = function()
		decide("allow_once")
	end, "Allow" },
}

local function what(): string
	if first:get() then
		return "waiting"
	end
	return if #working:get() > 0 then "working" else "resting"
end

-- The island: flush with the screen's top edge, its lower corners round.
-- (Round all round, its top tucked above the window's edge: only its lower
-- curve shows.)
local island = ui.button {
	h = 70,
	mt = ui.raw(-24, "the top corners tucked above the screen's edge"),
	pt = ui.raw(24, "the content below the tucked edge"),
	px = "$space.5",
	rounded = 22,
	bg = "$kx.sky.deep",
	hover = { bg = "$kx.sky.deep" },
	border = 1,
	border_color = function()
		return if first:get() then "$kx.attention.soft" else "$border.subtle"
	end,
	inset_shadow = function()
		return if first:get() then "$kx.lip.inset_strong" else "$kx.lip.inset"
	end,
	shadow = "$shadow.md",
	cursor = "pointer",
	label = "Keplorayx",
	on_click = focus,
	items = "center",
	ui.presence(what, function(kind: string): Node
		return if kind == "waiting" then waiting elseif kind == "working" then at_work else resting
	end, {
		enter = { opacity = { 0, 1 }, duration = 160 },
		exit = { opacity = { 1, 0 }, duration = 90 },
	}),
}

-- Hidden while something waits: a thin warm line at the edge.
local hint = ui.div {
	w = 96,
	h = 3,
	rounded_bl = 3,
	rounded_br = 3,
	bg = "$kx.attention",
	opacity = glow,
}

-- What shows: the island, the hint, or nothing.
local shown = ui.computed(function(): string?
	if near:get() then
		return "island"
	end
	return if first:get() then "hint" else nil
end)

return ui.row {
	-- Always night, whatever the app's theme: it sits on the screen's edge.
	theme = "orbit-night",
	w_full = true,
	h_full = true,
	justify = "center",
	items = "start",
	ui.presence(shown, function(kind: string): Node
		return if kind == "island" then island else hint
	end, {
		enter = { y = { -50, 0 }, opacity = { 0, 1 }, duration = 220, ease = kx.motion.ENTER },
		exit = { y = { 0, -50 }, opacity = { 1, 0 }, duration = 160, ease = kx.motion.EXIT },
	}),
}
