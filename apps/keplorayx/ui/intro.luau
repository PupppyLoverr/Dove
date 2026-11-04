--!strict
-- First Light's arrival, over the whole display (src/intro.rs). A deep sky
-- fades in; Keplorayx's mark settles at its centre as the wordmark
-- resolves. "Begin" (or Enter) opens setup hidden at the card's place,
-- then the orbit tightens into that window's rounded rect and the sky
-- clears around it: setup is simply there. Any other key skips to it.

ui.theme.strict("warn")
local kx = require("@kx")
kx.setup()

local W = (ui.shared("intro.width", 720) :: any):peek() :: number
local H = (ui.shared("intro.height", 520) :: any):peek() :: number
local radius = ui.shared("intro.radius", 0)

local root = ui.ref()
-- 0 dark, 1 the sky, 2 the mark and words, 3 Begin, 4 closing in, 5 gone.
local phase = ui.state("intro.phase", 0)
local reduced = kx.motion.reduced_now()

local function at(ms: number, f: () -> ())
	ui.after(if reduced then 0 else ms, f)
end

-- The sky, the mark, the words and the button arrive in turn.
at(60, function()
	phase:set(1)
end)
at(700, function()
	phase:set(2)
end)
at(1900, function()
	phase:set(3)
	ui.invoke("intro.interactive")
end)

local function size(): (number, number)
	local r: any = root
	local w = r.width or 1600
	local h = r.height or 1000
	return w, h
end

local leaving = false
local function begin()
	if leaving then
		return
	end
	leaving = true
	local vw, vh = size()
	local x, y = math.floor((vw - W) / 2), math.floor((vh - H) / 2)
	ui.invoke("intro.prepare", { x = x, y = y, w = W, h = H })
	phase:set(4)
	-- The ring has nearly closed onto the window: show it beneath the sky.
	at(520, function()
		ui.invoke("intro.reveal", ui.now())
	end)
	-- The sky has cleared: setup is the window now.
	at(1100, function()
		phase:set(5)
		ui.invoke("intro.done")
	end)
end

local function skip()
	if leaving then
		return
	end
	leaving = true
	ui.invoke("intro.done")
end

local sky_alpha = ui.computed(function(): number
	local p = phase:get()
	return if p == 0 then 0 elseif p >= 4 then 0 else 1
end)
local words = ui.computed(function(): number
	local p = phase:get()
	return if p >= 2 and p < 4 then 1 else 0
end)
local cta = ui.computed(function(): number
	return if phase:get() == 3 then 1 else 0
end)

-- The orbit: a ring that starts round about the mark and closes onto the
-- card's rect, its corner landing on the window's own.
local closing = ui.computed(function(): number
	return if phase:get() >= 4 then 1 else 0
end)
local ring_w = ui.computed(function(): number
	return if closing:get() == 1 then W else 168
end)
local ring_h = ui.computed(function(): number
	return if closing:get() == 1 then H else 168
end)
local ring_r = ui.computed(function(): number
	local r = radius:get()
	return if closing:get() == 1 then (if type(r) == "number" and r > 0 then r else 12) else 84
end)

local ease = kx.motion.ENTER
local function tw(state: any, ms: number): any
	return kx.motion.tween(state, { duration = ms, ease = ease })
end

-- A few fixed stars: the same sky every time.
local STARS = { { 0.12, 0.18, 2 }, { 0.82, 0.22, 1.5 }, { 0.68, 0.74, 2 }, { 0.22, 0.8, 1.5 }, { 0.5, 0.12, 1 }, { 0.9, 0.62, 1 }, { 0.32, 0.36, 1 } }
local function stars(): Node
	local dots: { Node } = {}
	for _, s in STARS do
		table.insert(dots, ui.div {
			absolute = true,
			left = ui.raw(`{s[1] * 100}%`, "a star's place in the sky"),
			top = ui.raw(`{s[2] * 100}%`, "a star's place in the sky"),
			w = s[3] * 2,
			h = s[3] * 2,
			rounded = "$radius.full",
			bg = "$text.primary",
			opacity = tw(words, 900),
		})
	end
	return ui.div { absolute = true, top = 0, left = 0, right = 0, bottom = 0, pointer_events = false, table.unpack(dots) }
end

return ui.column {
	-- First Light is always night.
	theme = "orbit-night",
	ref = root,
	w_full = true,
	h_full = true,
	items = "center",
	justify = "center",
	focusable = true,
	on_key = function(e: any)
		if e.key == "Enter" and phase:peek() >= 3 then
			begin()
		else
			skip()
		end
		return true
	end,
	-- The sky.
	ui.div {
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		radial = function()
			return { ui.theme.get("kx.sky.glow"), ui.theme.get("kx.sky.mid"), ui.theme.get("kx.sky.deep") }
		end,
		opacity = tw(sky_alpha, 700),
	},
	stars(),
	-- The orbit, closing onto the window.
	ui.div {
		absolute = true,
		w = tw(ring_w, 700),
		h = tw(ring_h, 700),
		rounded = tw(ring_r, 700),
		border = 1.5,
		border_color = "$accent",
		opacity = tw(ui.computed(function(): number
			local p = phase:get()
			-- Only as it closes in: at rest it would cross the words.
			return if p == 4 then 1 else 0
		end), 400),
		pointer_events = false,
	},
	-- The mark and the words.
	ui.column {
		items = "center",
		gap = "$space.4",
		opacity = tw(words, 600),
		y = tw(ui.computed(function(): number
			return if words:get() == 1 then 0 else 12
		end), 700),
		ui.column { color = "$accent", kx.Icon { name = "orbit", size = 64 } },
		ui.text {
			"Keplorayx",
			literal = true,
			font = "$kx.display",
			text_size = "$font.size.xxl",
			weight = "$font.weight.semibold",
			letter_spacing = -0.5,
			color = "$text.primary",
		},
		kx.Caption { "Every agent, in orbit around your work.", literal = true },
		ui.column {
			mt = "$space.4",
			opacity = tw(cta, 400),
			kx.Button { size = "lg", shape = "pill", on_click = begin, "Begin", kx.Icon { name = "arrow-right", size = 16 } },
		},
		kx.Hint { "Press any key to skip", literal = true, opacity = tw(cta, 600) },
	},
}
