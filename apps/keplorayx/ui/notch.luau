--!strict
-- The notch: a capsule over every window at the top centre of the screen.
-- At rest it is quiet: your working agents as stars, breathing in step.
-- When one needs you, its lip of light turns warm and it says who and why,
-- with Allow and Deny right there. Anything else on it brings the main
-- window forward on that thread (src/notch.rs).

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

local function star(t: store.Thread): Node
	return kx.AgentMark { agent = t.harness_id, size = 7, live = function()
		return true
	end }
end

local resting = ui.row {
	gap = "$space.2",
	items = "center",
	ui.column { color = "$accent", kx.Icon { name = "orbit", size = 14 } },
	ui.show(
		function()
			return #working:get() > 0
		end,
		ui.row {
			gap = "$space.1",
			items = "center",
			ui.each(working, star, {
				key = function(t: store.Thread)
					return t.id
				end,
				gap = "$space.1",
				items = "center",
			}),
			ui.text {
				function()
					local n = #working:get()
					return if n == 1 then "1 agent working" else `{n} agents working`
				end,
				literal = true,
				text_size = "$font.size.sm",
				color = "$text.secondary",
			},
		},
		-- Nothing working: the notch on its own, a quiet clock, and a click
		-- to ask (the launcher).
		ui.row {
			gap = "$space.2",
			items = "center",
			ui.text { clock, literal = true, text_size = "$font.size.sm", weight = "$font.weight.medium", tabular = true },
			ui.text { day, literal = true, text_size = "$font.size.sm", color = "$text.tertiary" },
			ui.div { w = 1, h = 12, bg = "$border.subtle" },
			ui.text { "Ask anything", literal = true, text_size = "$font.size.sm", color = "$text.tertiary" },
			ui.text { "Alt Space", literal = true, text_size = "$font.size.xs", color = "$text.tertiary", font = "$font.mono" },
		}
	),
}

local waiting = ui.row {
	gap = "$space.2",
	items = "center",
	ui.div { w = 7, h = 7, rounded = "$radius.full", bg = "$kx.attention", opacity = glow },
	ui.text {
		function()
			local w = first:get()
			return if w then `{w.harness} wants to run {w.tool}` else ""
		end,
		literal = true,
		truncate = true,
		max_w = 210,
		text_size = "$font.size.sm",
		weight = "$font.weight.medium",
	},
	kx.Button { tone = "ghost", size = "xs", shape = "pill", on_click = function()
		decide("deny")
	end, "Deny" },
	kx.Button { tone = "attention", size = "xs", shape = "pill", on_click = function()
		decide("allow_once")
	end, "Allow" },
}

return ui.row {
	-- Always night, whatever the app's theme: it sits over the screen's edge.
	theme = "orbit-night",
	w_full = true,
	h_full = true,
	justify = "center",
	items = "start",
	ui.button {
		h = 40,
		px = "$space.4",
		rounded = "$radius.full",
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
		ui.show(function()
			return first:get() ~= nil
		end, waiting, resting),
	},
}
