-- "Your agents, anywhere": an intro played over the desktop, in the manner
-- of Dia's. A sea at night decodes out of characters round the moon; it
-- spreads as the night turns to blue hour and to sunrise, clouds drifting in
-- at their own speeds; the agents rise from the horizon; then the words, and
-- a way in.
--
-- The plates and clouds are painted (art/), drawn as characters (`characters`:
-- a glyph a cell by the brightness there, over the picture, on the GPU as it
-- draws). Their shared shader parameters reveal them: a front moving out from
-- a point, cells ahead of it empty, those in its band a glyph alone and
-- scrambling, those behind it the picture, its edge ragged with characters.
-- Everything moves on the compositor or in those parameters: once started, no
-- script and no layout a frame.

local PIXEL = 2
local HORIZON = 0.62
-- The reveal (`shader_params` of every layer): front, band, ragged; centre.
local BAND, RAGGED = 0.36, 0.14
-- The agents Zeron runs (their marks from zeron.sh), as T3 Code's landing
-- page floats its harnesses: scattered loosely about the words, no two alike
-- in place, size or tilt, flying in from outside (`fx`, `fy`) and drifting
-- each on its own period. `x` is a fraction of the width from the middle, `y`
-- of the height from the top.
type Agent = {
	mark: string,
	x: number,
	y: number,
	s: number,
	rot: number,
	fx: number,
	fy: number,
	d: number,
	dur: number,
	dx: number,
	dy: number,
	wob: number,
}
local AGENTS: { Agent } = {
	{
		mark = "claude-mark",
		x = -0.33,
		y = 0.2,
		s = 96,
		rot = -8,
		fx = -160,
		fy = -60,
		d = 0.38,
		dur = 10.5,
		dx = 4,
		dy = -12,
		wob = -2,
	},
	{
		mark = "openai-mark",
		x = 0.28,
		y = 0.12,
		s = 84,
		rot = 7,
		fx = 140,
		fy = -90,
		d = 0.46,
		dur = 9.2,
		dx = -4,
		dy = -10,
		wob = 2,
	},
	{
		mark = "opencode-mark-dark",
		x = -0.08,
		y = 0.1,
		s = 48,
		rot = 6,
		fx = -30,
		fy = -100,
		d = 0.62,
		dur = 8.5,
		dx = 2,
		dy = -8,
		wob = 3,
	},
	{
		mark = "antigravity-mark",
		x = 0.11,
		y = 0.21,
		s = 56,
		rot = -5,
		fx = 50,
		fy = -80,
		d = 0.58,
		dur = 9.8,
		dx = -2,
		dy = -9,
		wob = -3,
	},
	{
		mark = "devin-mark",
		x = -0.44,
		y = 0.46,
		s = 64,
		rot = 9,
		fx = -200,
		fy = 10,
		d = 0.66,
		dur = 10.1,
		dx = 3,
		dy = -8,
		wob = 2,
	},
	{
		mark = "pi-mark",
		x = 0.21,
		y = 0.71,
		s = 60,
		rot = -7,
		fx = 80,
		fy = 120,
		d = 0.5,
		dur = 8.8,
		dx = -3,
		dy = -10,
		wob = -2,
	},
	{
		mark = "cursor-mark",
		x = 0.41,
		y = 0.4,
		s = 76,
		rot = -5,
		fx = 190,
		fy = -20,
		d = 0.7,
		dur = 7.8,
		dx = -3,
		dy = -11,
		wob = 3,
	},
	{
		mark = "grok-mark",
		x = -0.17,
		y = 0.67,
		s = 68,
		rot = 4,
		fx = -90,
		fy = 110,
		d = 0.54,
		dur = 11.5,
		dx = 3,
		dy = -9,
		wob = -3,
	},
	{
		mark = "hermes-mark",
		x = 0.39,
		y = 0.6,
		s = 52,
		rot = 3,
		fx = 170,
		fy = 90,
		d = 0.74,
		dur = 9.5,
		dx = 2,
		dy = -7,
		wob = 2,
	},
	{
		mark = "muse-mark",
		x = -0.36,
		y = 0.82,
		s = 80,
		rot = -4,
		fx = -140,
		fy = 140,
		d = 0.78,
		dur = 10.8,
		dx = -2,
		dy = -8,
		wob = -2,
	},
}
-- When the agents come, in ms from the start.
local AGENTS_AT = 4200

-- The button's rim, a beam running round it.
local beam = ui.shader([[
fn rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
	let q = abs(p) - b + vec2<f32>(r);
	return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}
fn fill(input: FillInput) -> vec4<f32> {
	let half = input.size * 0.5;
	let p = input.p - half;
	let d = rounded_box(p, half, half.y);
	let rim = 1.0 - smoothstep(0.0, 1.2, abs(d + 0.9));
	let turn = fract(atan2(p.y, p.x) / 6.2831853 + 0.5 - input.time * 0.22);
	let head = pow(turn, 10.0);
	let col = mix(vec3<f32>(0.62, 0.45, 0.95), vec3<f32>(1.0, 0.62, 0.38), head);
	return vec4<f32>(col, rim * (0.15 + 0.85 * head));
}]])

local started = false
local leaving = false

local function cover(src: string, id: string?, opacity: number?): Node
	return ui.image {
		id = id,
		class = "scene",
		src = src,
		w_full = true,
		h_full = true,
		fit = "cover",
		characters = PIXEL,
		-- Nothing shown yet: the front at the moon, on the horizon.
		shader_params = { { 0, BAND, RAGGED, 0 }, { 0, HORIZON - 0.5, 0, 0 } },
		opacity = opacity,
	}
end

-- A layer of the scene, moved as one (the camera, a bank of cloud).
local function layer(id: string, ...: Node): Node
	return ui.overlay {
		id = id,
		absolute = true,
		left = "-6%",
		right = "-6%",
		top = 0,
		bottom = 0,
		...,
	}
end

-- An agent's card: white, its mark in its own colours at 56%; the
-- outer box flies in, the inner drifts.
local function agent(i: number, a: Agent): Node
	return ui.div {
		id = `agent{i}`,
		class = "agent",
		absolute = true,
		left = `{50 + a.x * 100}%`,
		top = `{a.y * 100}%`,
		w = a.s,
		h = a.s,
		ml = -a.s / 2,
		mt = -a.s / 2,
		opacity = 0,
		ui.column {
			id = `drift{i}`,
			w = a.s,
			h = a.s,
			rounded = a.s * 0.25,
			items = "center",
			justify = "center",
			gradient = { angle = 135, stops = { "#ffffff", "#f6f6f8" } },
			border = 1,
			border_color = "#ffffffcc",
			shadow = { y = 20, blur = 48, color = "#1a0f3a73" },
			inset_shadow = { y = -2, color = "#0000000d" },
			ui.svg {
				src = `icons/{a.mark}.svg`,
				w = a.s * 0.56,
				h = a.s * 0.56,
			},
		},
	}
end

local agents: { Node } = {}
for i, a in AGENTS do
	table.insert(agents, agent(i, a))
end

-- The scene decodes out of characters round the moon, then spreads to fill
-- most of the window, its centre gliding up to the middle.
local function open(_w: number, _h: number)
	anime.animate(".scene", { shaderParam0 = { 0, 0.5 }, duration = 1800, ease = "outCubic" })
	anime.animate(".scene", {
		shaderParam0 = 1.3,
		shaderParam5 = 0,
		delay = 1800,
		duration = 2800,
		ease = "inOutCubic",
	})
end

-- Night to blue hour to sunrise, the camera drifting all the while.
local function timelapse(w: number)
	anime.animate("#world", { x = { w * 0.03, -w * 0.03 }, duration = 11000, ease = "outSine" })
	anime.animate("#bluehour", { opacity = { 0, 1 }, delay = 1700, duration = 1700, ease = "inOutSine" })
	anime.animate("#sunrise", { opacity = { 0, 1 }, delay = 3500, duration = 2000, ease = "inOutSine" })
	-- Nearer, faster: the banks come in from the sides as the light does.
	anime.animate("#clouds-left", {
		x = { -w * 0.16, 0 },
		opacity = { 0, 1 },
		delay = 2400,
		duration = 4200,
		ease = "outCubic",
	})
	anime.animate("#clouds-right", {
		x = { w * 0.16, 0 },
		opacity = { 0, 1 },
		delay = 2400,
		duration = 4200,
		ease = "outCubic",
	})
	anime.animate("#clouds-mid", {
		x = { w * 0.05, -w * 0.02 },
		opacity = { 0, 1 },
		delay = 3000,
		duration = 8000,
		ease = "outSine",
	})
end

-- The agents fly in from outside their places, turning and growing into them,
-- each in its turn; then each drifts on its own period.
local function rise(_w: number, _h: number)
	for i, a in AGENTS do
		local at = AGENTS_AT + a.d * 1000
		anime.animate(`#agent{i}`, {
			opacity = { 0, 1 },
			x = { a.fx, 0 },
			y = { a.fy, 0 },
			rotate = { a.rot + 24, a.rot },
			scale = { 0.6, 1 },
			delay = at,
			duration = 1000,
			ease = "out(4)",
		})
		anime.animate(`#drift{i}`, {
			x = { 0, a.dx },
			y = { 0, a.dy },
			rotate = { 0, a.wob },
			delay = at + 1000,
			duration = a.dur * 500,
			alternate = true,
			loop = true,
			ease = "inOutSine",
		})
	end
end

-- The words, then the way in (and the pointer, for it).
local function words()
	anime.animate(".word", {
		opacity = { 0, 1 },
		y = { 18, 0 },
		blur = { 12, 0 },
		delay = anime.stagger(140, { start = 6300 }),
		duration = 1100,
		ease = "outExpo",
	})
	anime.animate("#cta", {
		opacity = { 0, 1 },
		y = { 10, 0 },
		delay = 7400,
		duration = 900,
		ease = "outExpo",
		onBegin = function()
			ui.invoke("interactive")
		end,
	})
end

local function start(w: number, h: number)
	open(w, h)
	timelapse(w)
	rise(w, h)
	words()
end

-- Out: the agents home, the words gone, the circle closed, then the window.
local function leave()
	if leaving then
		return
	end
	leaving = true
	anime.animate(".agent", {
		opacity = 0,
		scale = 0.3,
		delay = anime.stagger(40),
		duration = 500,
		ease = "inBack",
	})
	anime.animate(".word, #cta", { opacity = 0, y = -10, duration = 400, ease = "inQuad" })
	-- Back into characters, from the edge in.
	anime.animate(".scene", {
		shaderParam0 = -BAND,
		delay = 250,
		duration = 1100,
		ease = "inCubic",
		onComplete = function()
			ui.invoke("close")
		end,
	})
end

-- Text over the bright sea: a dusk-coloured blur of itself behind it, as a
-- text shadow (drawn once into its own layer, then only composited).
local function shadowed(props: { [any]: any }): Node
	local shadow = table.clone(props)
	shadow.color = "#2a1250"
	shadow.blur = 10
	shadow.opacity = 0.7
	shadow.absolute = true
	shadow.left = 0
	shadow.top = 2
	return ui.overlay { class = "word", opacity = 0, ui.text(shadow), ui.text(props) }
end

local function word(text: string, italic: boolean?): Node
	return shadowed {
		text,
		font = "Instrument Serif",
		italic = italic,
		text_size = 80,
		letter_spacing = -1,
		color = "#ffffff",
	}
end

local button = ui.button {
	id = "cta",
	opacity = 0,
	h = 46,
	px = 28,
	rounded = 23,
	bg = "#fffffff2",
	hover = { bg = "#ffffff" },
	shadow = { y = 10, blur = 26, color = "#3a1d6a66" },
	bg_shader = beam,
	cursor = "pointer",
	on_click = leave,
	ui.text { "Get started", font = "Geist", text_size = 15, weight = 500, color = "#2a1452" },
}

return ui.overlay {
	w_full = true,
	h_full = true,
	on_key = function(e)
		if e.key == "Escape" then
			leave()
			return true
		end
		return false
	end,
	-- The sequence starts once the window's size is known.
	on_resize = function(s)
		if not started and s.w > 0 and s.h > 0 then
			started = true
			start(s.w, s.h)
		end
	end,
	-- The scene, revealed by its own characters.
	ui.overlay {
		id = "portal",
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		layer(
			"world",
			cover("art/plate-1-night.jpg"),
			cover("art/plate-2-bluehour.jpg", "bluehour", 0),
			cover("art/plate-3-sunrise.jpg", "sunrise", 0)
		),
		layer("clouds-mid", cover("art/clouds-mid.png")),
		layer("clouds-left", cover("art/clouds-left.png")),
		layer("clouds-right", cover("art/clouds-right.png")),
		ui.overlay { absolute = true, left = 0, right = 0, top = 0, bottom = 0, table.unpack(agents) },
		-- The title in the deep blue high up, where white reads best.
		ui.row {
			absolute = true,
			left = 0,
			right = 0,
			top = "35%",
			justify = "center",
			gap = 16,
			word("Your"),
			word("agents,"),
			word("anywhere.", true),
		},
		-- The way in, on the sea under the sun.
		ui.column { absolute = true, left = 0, right = 0, top = "48%", items = "center", button },
	},
}
