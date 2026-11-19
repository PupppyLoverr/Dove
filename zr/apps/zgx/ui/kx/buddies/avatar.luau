--!strict
-- A buddy's face: a soft shape in its colour, two dark eyes that look at
-- you (they follow the
-- pointer when it comes near), and, once it's big enough, a mouth and
-- cheeks; something on top only when chosen: an antenna, a sprout, a halo
-- or a spark.
--
--   Avatar { look = { shape, color, face, extra }, seed = buddy.id, size = 28,
--            standing = function(): string? … end, live = true,
--            badge = function(): string? … end }   -- "star" | "dot" | "done"
--
-- `look` may leave any part out: it's chosen from `seed`, so a buddy with no
-- look of its own still has one that stays the same. `standing` changes it:
-- while it works it bobs, breathes and its eyes scan (its antenna blinks,
-- its spark twinkles); waiting on you it looks up; done, it smiles with its
-- eyes; failed, they close. `live`: it blinks now and then, and moves.
-- Reduce motion keeps it still.
--
--   Avatar.Picker { look = State<Look>, on_change = fn(look) }
--       shapes, colours, eyes and extras to choose from, and Randomize
--   Avatar.track(e) / Avatar.untrack()
--       for a surface to hand on where the pointer is, so faces on it
--       follow it from afar (each face follows it over itself anyway)

export type Look = { shape: string?, color: string?, face: string?, extra: string?, image: string? }
export type Props = {
	look: Look? | (() -> Look?),
	seed: string?,
	size: number?,
	standing: (() -> string?)?,
	live: boolean?,
	-- (Moves while it works: on unless false. Still with Reduce motion on.)
	breathe: boolean?,
	-- (Its eyes follow the pointer: on unless false.)
	follow: boolean?,
	badge: (() -> string?)?,
}
type Full = { shape: string, color: string, face: string, extra: string, image: string? }
type Drawn = { shut: boolean?, awake: boolean?, scale: number?, bob: number?, gx: number?, gy: number?, t: number? }

local motion = require("@ds/motion")

local M = {}

M.SHAPES = { "circle", "cloud", "diamond", "star4", "clover", "heart", "puff", "drop", "bean", "triangle", "pentagon", "splat", "hexagon" }
M.FACES = { "open", "round", "calm", "happy", "sleepy", "wink" }
M.EXTRAS = { "none", "antenna", "sprout", "halo", "spark" }
M.COLORS = {
	{ id = "snow", hex = "#ecebe7" },
	{ id = "bronze", hex = "#c08a63" },
	{ id = "red", hex = "#e8636f" },
	{ id = "orange", hex = "#ef8a52" },
	{ id = "amber", hex = "#e9b35a" },
	{ id = "green", hex = "#5cbf8f" },
	{ id = "mint", hex = "#5cc6bb" },
	{ id = "blue", hex = "#6f93ec" },
	{ id = "violet", hex = "#9a85e6" },
	{ id = "rose", hex = "#e07fae" },
	{ id = "slate", hex = "#8a8d96" },
}

-- (Looks saved before these shapes and colours: the nearest of the new.)
local OLD_SHAPES = { squircle = "diamond", soft = "circle", tall = "drop", wide = "bean", stone = "pentagon" }
local OLD_COLORS = { ink = "slate", sky = "blue", sun = "amber", plum = "violet" }
local OLD_FACES = { up = "open" }
local OLD_EXTRAS = { horns = "none", ears = "none" }
-- (What a seed picks from, when nothing's chosen: the friendlier shapes.)
local SEEDED = { "circle", "cloud", "puff", "bean", "drop", "clover", "hexagon", "heart" }

local INK = "#1b1a22"
local WHITE = "#ffffff"
local GOLD = "#fcd34d"

local function hash(s: string): number
	local h = 7
	for i = 1, #s do
		h = (h * 31 + string.byte(s, i)) % 2147483647
	end
	return h
end

-- A look with every part said: what's given, else what its seed chooses.
function M.resolve(look: Look?, seed: string?): Full
	local h = hash(seed or "buddy")
	local l: Look = look or {}
	local shape = l.shape and (OLD_SHAPES[l.shape] or l.shape)
	local color = l.color and (OLD_COLORS[l.color] or l.color)
	return {
		shape = shape or SEEDED[h % #SEEDED + 1],
		-- (Never snow or slate by chance: those are chosen.)
		color = color or M.COLORS[(h // 7) % (#M.COLORS - 2) + 2].id,
		face = (l.face and (OLD_FACES[l.face] or l.face)) or M.FACES[(h // 91) % 2 + 1],
		extra = (l.extra and (OLD_EXTRAS[l.extra] or l.extra)) or "none",
		-- (A picture of its own, uploaded, in place of the drawn face.)
		image = if type(l.image) == "string" and l.image ~= "" then l.image else nil,
	}
end

function M.hex(id: string): string
	local want = OLD_COLORS[id] or id
	for _, c in M.COLORS do
		if c.id == want then
			return c.hex
		end
	end
	return M.COLORS[7].hex
end

-- A colour mixed toward black (`to` 0) or white (255) by `k`.
local function mix(hex: string, to: number, k: number): string
	local out = "#"
	for i = 0, 2 do
		local v = tonumber(string.sub(hex, 2 + i * 2, 3 + i * 2), 16) or 0
		out ..= string.format("%02x", math.floor(v + (to - v) * k + 0.5))
	end
	return out
end

-- A look's colours for tinting what's around it: its fill, the ink on it,
-- and a soft wash of it.
function M.resolve_color(look: Look?, seed: string?): { fill: string, ink: string, soft: string }
	local hex = M.hex(M.resolve(look, seed).color)
	return { fill = hex, ink = INK, soft = hex .. "33" }
end

-- Shapes as radius by angle (1 = the full half-size), or rounded polygons.
local TAU = math.pi * 2
local POLAR: { [string]: (a: number) -> number } = {
	circle = function(_a)
		return 1
	end,
	cloud = function(a)
		return 0.86 + 0.14 * math.abs(math.cos(a * 2.5))
	end,
	star4 = function(a)
		local c = math.cos(a * 4)
		return 0.74 + 0.26 * c * math.abs(c) ^ 0.2
	end,
	clover = function(a)
		return 0.82 + 0.18 * math.abs(math.cos(a * 2 + math.pi / 4)) ^ 0.6
	end,
	puff = function(a)
		return 0.86 + 0.14 * math.abs(math.cos(a * 4))
	end,
	splat = function(a)
		local c = math.cos(a * 6)
		return 0.72 + 0.28 * c * math.abs(c) ^ 0.3
	end,
	drop = function(a)
		-- (An egg, its point up and to the right.)
		return 0.78 + 0.22 * math.cos(a + math.pi * 0.7)
	end,
}
local POLY: { [string]: { sides: number, turn: number, round: number } } = {
	diamond = { sides = 4, turn = math.rad(30), round = 0.3 },
	triangle = { sides = 3, turn = math.rad(-12), round = 0.32 },
	pentagon = { sides = 5, turn = math.rad(-16), round = 0.24 },
	hexagon = { sides = 6, turn = math.rad(14), round = 0.22 },
}

local function polar_path(cx: number, cy: number, r: number, f: (a: number) -> number, turn: number): any
	local n = 96
	local pts = {}
	for i = 0, n - 1 do
		local a = i / n * TAU
		local rr = r * f(a)
		table.insert(pts, { cx + math.cos(a + turn) * rr, cy + math.sin(a + turn) * rr })
	end
	local p = ui.path()
	local function mid(i: number): (number, number)
		local a, b = pts[(i - 1) % n + 1], pts[i % n + 1]
		return (a[1] + b[1]) / 2, (a[2] + b[2]) / 2
	end
	local mx, my = mid(1)
	p:move(mx, my)
	for i = 2, n + 1 do
		local c = pts[(i - 1) % n + 1]
		local x, y = mid(i)
		p:quad(c[1], c[2], x, y)
	end
	p:close()
	return p
end

local function poly_path(cx: number, cy: number, r: number, sides: number, turn: number, round: number): any
	local pts = {}
	for i = 0, sides - 1 do
		local a = turn - math.pi / 2 + i / sides * TAU
		table.insert(pts, { cx + math.cos(a) * r, cy + math.sin(a) * r })
	end
	local p = ui.path()
	local function lerp(a: { number }, b: { number }, t: number): (number, number)
		return a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t
	end
	for i = 1, sides do
		local prev = pts[(i - 2) % sides + 1]
		local cur = pts[i]
		local nxt = pts[i % sides + 1]
		local ax, ay = lerp(cur, prev, round)
		local bx, by = lerp(cur, nxt, round)
		if i == 1 then
			p:move(ax, ay)
		else
			p:line(ax, ay)
		end
		p:quad(cur[1], cur[2], bx, by)
	end
	p:close()
	return p
end

local function heart_path(cx: number, cy: number, r: number): any
	-- (A plump one, tipped a little: the classic curve, rounded toward a circle.)
	local f = function(a: number): number
		return 1
	end
	local n = 96
	local pts = {}
	local turn = math.rad(-14)
	for i = 0, n - 1 do
		local t = i / n * TAU
		local x = 16 * math.sin(t) ^ 3
		local y = -(13 * math.cos(t) - 5 * math.cos(2 * t) - 2 * math.cos(3 * t) - math.cos(4 * t))
		x, y = x / 17, (y + 2) / 17
		-- (Halfway to a circle: plumper.)
		local d = math.sqrt(x * x + y * y)
		if d > 0 then
			local k = 0.55 + 0.45 / d
			x, y = x * (0.55 + 0.45 * k), y * (0.55 + 0.45 * k)
		end
		local rx = x * math.cos(turn) - y * math.sin(turn)
		local ry = x * math.sin(turn) + y * math.cos(turn)
		table.insert(pts, { cx + rx * r * f(t), cy + ry * r })
	end
	local p = ui.path()
	p:move(pts[1][1], pts[1][2])
	for i = 2, n do
		p:line(pts[i][1], pts[i][2])
	end
	p:close()
	return p
end

local function body_path(shape: string, cx: number, cy: number, r: number): any
	if shape == "heart" then
		return heart_path(cx, cy, r)
	elseif shape == "bean" then
		-- (A tipped pill: an ellipse, flatter, leaning.)
		return polar_path(cx, cy, r, function(a: number): number
			local ex, ey = math.cos(a), math.sin(a) / 0.7
			return 1 / math.sqrt(ex * ex + ey * ey)
		end, math.rad(-18))
	end
	local poly = POLY[shape]
	if poly then
		return poly_path(cx, cy, r * 1.06, poly.sides, poly.turn, poly.round)
	end
	return polar_path(cx, cy, r, POLAR[shape] or POLAR.circle, 0)
end

local function ellipse(cx: number, cy: number, rx: number, ry: number, turn: number?): any
	local n = 32
	local ct, st = math.cos(turn or 0), math.sin(turn or 0)
	local p = ui.path()
	for i = 0, n - 1 do
		local a = i / n * TAU
		local x, y = math.cos(a) * rx, math.sin(a) * ry
		local px, py = cx + x * ct - y * st, cy + x * st + y * ct
		if i == 0 then
			p:move(px, py)
		else
			p:line(px, py)
		end
	end
	p:close()
	return p
end

local function leaf(c: any, x: number, y: number, side: number, len: number, color: string)
	local tx, ty = x + side * len * 0.9, y - len * 0.5
	local p = ui.path()
	p:move(x, y)
	p:quad(x + side * len * 0.3, y - len * 0.8, tx, ty)
	p:quad(x + side * len * 0.65, y + len * 0.1, x, y)
	p:close()
	c:fill(p, color)
end

local function working(s: string?): boolean
	return s == "working" or s == "connecting" or s == "plan"
end

-- An arc of a closed or smiling eye: `up` bows it upward (^), else down (‿).
local function arc(c: any, x: number, y: number, s: number, up: boolean, width: number)
	local p = ui.path()
	p:move(x - s, y)
	p:quad(x, y + (if up then -s * 1.1 else s * 0.8), x + s, y)
	c:stroke(p, INK, width, { cap = "round", join = "round" })
end

-- One beat for every face at work: it counts only while a face is
-- working, and only working faces read it, so one at rest never redraws for it.
local beat = ui.state("kx_avatar_beat", 0)
local working_faces = 0
local stop_beat: (() -> ())? = nil

local function beat_start()
	working_faces += 1
	if not stop_beat then
		stop_beat = ui.every(50, function()
			beat:set(beat:peek() + 1)
		end)
	end
end

local function beat_stop()
	working_faces -= 1
	if working_faces <= 0 then
		working_faces = 0
		if stop_beat then
			stop_beat()
			stop_beat = nil
		end
	end
end

-- Body outlines kept by shape and place: rebuilt only when either changes.
local outlines: { [string]: any } = {}
local outline_count = 0

local function cached(shape: string, cx: number, cy: number, r: number): any
	local key = `{shape}|{math.floor(cx * 4)}|{math.floor(cy * 4)}|{math.floor(r * 8)}`
	local p = outlines[key]
	if not p then
		if outline_count > 400 then
			outlines = {}
			outline_count = 0
		end
		p = body_path(shape, cx, cy, r)
		outlines[key] = p
		outline_count += 1
	end
	return p
end

local function draw(c: any, w: number, h: number, look: Full, standing: string?, o: Drawn?)
	local d: Drawn = o or {}
	local size = math.min(w, h)
	local extra = look.extra
	local topped = extra == "antenna" or extra == "sprout" or extra == "halo"
	local r = size * (if topped then 0.41 else 0.47) * (d.scale or 1)
	local cx = w / 2
	local cy = h / 2 + (if topped then size * 0.07 else 0) + (d.bob or 0) * r
	local fill = M.hex(look.color)
	local t = d.t or 0
	local busy = working(standing)

	-- What grows from its top, behind it.
	if extra == "antenna" then
		local tx, ty = cx + r * 0.16, cy - r * 1.26
		c:line(cx, cy - r * 0.7, tx, ty, mix(fill, 0, 0.18), math.max(1, r * 0.075), { cap = "round" })
		local lit = if busy and math.sin(t * 0.3) > 0 then "#ffffff" else mix(fill, 255, 0.45)
		c:circle(tx, ty, r * 0.13, lit)
	elseif extra == "sprout" then
		local sway = if busy then math.sin(t * 0.15) * r * 0.05 else 0
		local tx, ty = cx + sway, cy - r * 1.12
		c:line(cx, cy - r * 0.75, tx, ty, "#3f9d5a", math.max(1, r * 0.07), { cap = "round" })
		leaf(c, tx, ty, -1, r * 0.38, "#4cb86b")
		leaf(c, tx, ty, 1, r * 0.3, "#6fd08a")
	end

	-- Its body: one flat shape in its colour.
	c:fill(cached(look.shape, cx, cy, r), fill)

	-- Its eyes: two pills a little above the middle and toward the right,
	-- leaning, as if it looked past you; they follow where it looks.
	local face = look.face
	if standing == "done" then
		face = "happy"
	end
	local ew = math.max(1.5, r * 0.17)
	local len = r * 0.66
	if face == "calm" then
		len = r * 0.3
	elseif face == "round" then
		len = ew * 1.05
	end
	local gx, gy = d.gx or 0, d.gy or 0
	local ex, ey = cx + r * 0.16 + gx * r * 0.13, cy - r * 0.16 + gy * r * 0.12
	local gap = r * 0.42
	local lean = math.rad(12)
	local closed = d.shut == true or standing == "failed" or (face == "sleepy" and not d.awake)
	for i, side in { -1, 1 } do
		local x = ex + side * gap / 2
		local y = ey + side * gap * 0.12
		if face == "happy" and not closed then
			-- (^^: two little arches.)
			local s = ew * 1.6
			local p = ui.path()
			p:move(x - s, y + s * 0.5)
			p:quad(x, y - s * 1.1, x + s, y + s * 0.5)
			c:stroke(p, INK, ew * 0.8, { cap = "round", join = "round" })
		else
			local l = len
			if closed or (face == "wink" and i == 2) then
				l = ew * 0.2
			end
			local dx, dy = math.sin(lean) * (l - ew) / 2, math.cos(lean) * (l - ew) / 2
			local half = math.max(dy, 0.01)
			c:line(x + dx, y - half, x - dx, y + half, INK, ew, { cap = "round" })
		end
	end

	-- What floats over it.
	if extra == "halo" then
		local lift = if busy then math.sin(t * 0.12) * r * 0.04 else 0
		c:stroke(ellipse(cx, cy - r * 1.14 + lift, r * 0.4, r * 0.1), "#f3c955", math.max(1.1, r * 0.07), {})
	elseif extra == "spark" then
		local k = if busy then 0.85 + 0.25 * math.abs(math.sin(t * 0.18)) else 1
		c:fill(polar_path(cx + r * 0.86, cy - r * 0.8, r * 0.22 * k, POLAR.star4, if busy then t * 0.05 else 0), "#f3c955")
	end
end

-- Where the pointer is, as a surface last said (or false: not near any).
local pointer = ui.state("kx_avatar_pointer", false :: any)

function M.track(e: any)
	if not (e and e.x) then
		return
	end
	-- (Only when it moved: a redraw hands on where it is again, unmoved,
	-- and setting it then would go round for ever.)
	local was = pointer:peek()
	if was and math.abs(was.x - e.x) < 1 and math.abs(was.y - e.y) < 1 then
		return
	end
	pointer:set({ x = e.x, y = e.y })
end

function M.untrack()
	if pointer:peek() then
		pointer:set(false)
	end
end

local View = ui.component(function(props: Props): Node
	local size = props.size or 28
	local ref = ui.ref()
	local function standing(): string?
		return if props.standing then props.standing() else nil
	end
	-- (Its blink: shut for a moment every few seconds, each on its own beat —
	-- but only while the window is someone's to look at. Left running, six of
	-- these on a buddy roster drew 2.4 frames a second of 0.0 megapixels for
	-- ever: the window never went quiet, and idle cost a core's worth of
	-- presents for a blink nobody could see.)
	local shut = ui.use_state(false)
	if props.live then
		local beat = 3400 + hash(props.seed or "") % 2600
		local stop_blink: (() -> ())? = nil
		local function watched(): boolean
			-- (`ui.window.active` follows the window being the active one;
			-- a host that doesn't say reads as watched.)
			local on, active = pcall(function()
				return ui.window.active:get()
			end)
			local shows, shown = pcall(function()
				return ui.window.shown:get()
			end)
			return (not on or active ~= false) and (not shows or shown ~= false)
		end
		ui.effect(function()
			if stop_blink then
				stop_blink()
				stop_blink = nil
			end
			-- (A face that stopped being watched opens its eyes.)
			if shut:peek() then
				shut:set(false)
			end
			if not watched() then
				return
			end
			stop_blink = ui.every(beat, function()
				shut:set(true)
				ui.after(130, function()
					shut:set(false)
				end)
			end)
		end)
		ui.on_cleanup(function()
			if stop_blink then
				stop_blink()
				stop_blink = nil
			end
		end)
	end
	-- (Its motion while it works: counted on the one shared beat, from
	-- when it starts working to when it stops.)
	local moves = props.live == true and props.breathe ~= false
	if moves then
		local counted = false
		ui.effect(function()
			local w = working(standing())
			if w ~= counted then
				counted = w
				if w then
					beat_start()
				else
					beat_stop()
				end
			end
		end)
		ui.on_cleanup(function()
			if counted then
				counted = false
				beat_stop()
			end
		end)
	end
	local canvas = ui.canvas {
		w = size,
		h = size,
		shrink = 0,
		draw = function(c, w: number, h: number)
			local given = props.look
			local look = M.resolve(if type(given) == "function" then given() else given, props.seed)
			local s = standing()
			local t = if moves and working(s) and not motion.reduced_now() then beat:get() else 0
			-- Where it looks: at the pointer when it's near, else as it's doing.
			local gx, gy, seen = 0, 0, false
			local p = if props.follow ~= false then pointer:get() else nil
			if p then
				local box = ref:peek()
				if box and box.w then
					local dx, dy = p.x - (box.x + box.w / 2), p.y - (box.y + box.h / 2)
					local dist = math.sqrt(dx * dx + dy * dy)
					if dist < math.max(420, size * 6) and dist > 0.5 then
						local k = math.min(1, dist / math.max(size * 0.7, 20))
						gx, gy, seen = dx / dist * k, dy / dist * k, true
					end
				end
			end
			if not seen then
				if t > 0 then
					gx, gy = math.sin(t * 0.06) * 0.7, 0.1
				elseif s == "input" or s == "approval" then
					gx, gy = 0.1, -0.85
				end
			end
			draw(c, w, h, look, s, {
				shut = shut:get(),
				awake = seen,
				scale = if t == 0 then 1 else 0.98 + 0.02 * math.sin(t * 0.12),
				bob = if t == 0 then 0 else math.sin(t * 0.12 + 1.2) * 0.035,
				gx = gx,
				gy = gy,
				t = t,
			})
		end,
	}
	local b = math.max(8, math.floor(size * 0.3))
	return ui.div {
		ref = ref,
		w = size,
		h = size,
		shrink = 0,
		on_pointer_move = M.track,
		-- (Its picture when it has one, else its drawn face.)
		ui.presence(function(): string
			local given = props.look
			local l = if type(given) == "function" then given() else given
			return if l and type(l.image) == "string" then l.image else ""
		end, function(src: string): Node
			if src == "" then
				return canvas
			end
			return ui.image { src = src, w = size, h = size, fit = "cover", rounded = "$radius.full" }
		end, { w = size, h = size }),
		if props.badge
			then ui.div {
				absolute = true,
				right = -1,
				bottom = -1,
				w = b,
				h = b,
				rounded = "$radius.full",
				items = "center",
				justify = "center",
				hidden = function(): boolean
					return props.badge ~= nil and props.badge() == nil
				end,
				bg = function(): string
					local kind = props.badge and props.badge()
					return ui.raw(if kind == "done" then "#10b981" else "#f97316", "the badge colours of the avatar palette")
				end,
				border = 2,
				border_color = "$background",
				ui.text {
					function(): string
						return if props.badge and props.badge() == "star" then "★" else ""
					end,
					literal = true,
					color = "$accent.text",
					text_size = ui.raw(math.max(6, b - 6), "a glyph sized to its badge"),
				},
			}
			else nil,
	}
end)

local function label(text: string): Node
	return ui.text { text, literal = true, color = "$text.tertiary", text_size = "$font.size.xs", weight = "$font.weight.medium" }
end

-- Shapes, colours, eyes and extras to choose from, each drawn in what's
-- chosen; `look` holds the choice. Randomize rolls a few before it lands.
M.Picker = ui.component(function(props: { look: State<Look>, seed: string?, on_change: ((Look) -> ())? }): Node
	local function current(): Full
		return M.resolve(props.look:get(), props.seed)
	end
	local function pick(change: Look)
		local cur = M.resolve(props.look:peek(), props.seed)
		local next: Look = {
			shape = change.shape or cur.shape,
			color = change.color or cur.color,
			face = change.face or cur.face,
			extra = change.extra or cur.extra,
			-- (Choosing any part of the drawn face sets the picture aside.)
			image = change.image,
		}
		props.look:set(next)
		if props.on_change then
			props.on_change(next)
		end
	end
	local rolling = ui.use_state(false)
	local function randomize()
		if rolling:peek() then
			return
		end
		local steps = if motion.reduced_now() then 1 else 8
		local n = 0
		local function roll()
			n += 1
			local next: Look = {
				shape = M.SHAPES[math.random(1, #M.SHAPES)],
				color = M.COLORS[math.random(2, #M.COLORS)].id,
				face = M.FACES[math.random(1, 4)],
				extra = if math.random() < 0.55 then "none" else M.EXTRAS[math.random(2, #M.EXTRAS)],
			}
			if n < steps then
				props.look:set(next)
				ui.after(40 + n * 14, roll)
			else
				rolling:set(false)
				pick(next)
			end
		end
		rolling:set(true)
		roll()
	end
	local function swatch(key: string, value: string, sel: () -> boolean, w: number, child: Node): Node
		return ui.button {
			label = `{key} {value}`,
			w = w,
			h = w,
			rounded = "$radius.full",
			items = "center",
			justify = "center",
			bg = function(): string
				return if sel() then "$hover" else "#00000000"
			end,
			border = 2,
			border_color = function(): string
				return if sel() then "$text.secondary" else "#00000000"
			end,
			hover = { bg = "$hover" },
			on_click = function()
				pick({ [key] = value } :: any)
			end,
			child,
		}
	end
	local function drawn(look: (Full) -> Full): Node
		return ui.canvas {
			w = 34,
			h = 34,
			draw = function(c, w: number, h: number)
				draw(c, w, h, look(current()), nil, { gx = 0.2, gy = 0.15 })
			end,
		}
	end
	local function row(): { [any]: any }
		return { w_full = true, flex_wrap = true, gap = "$space.1" }
	end
	local shapes, colors, faces, extras = row(), row(), row(), row()
	for _, s in M.SHAPES do
		table.insert(shapes, swatch("shape", s, function(): boolean
			return current().shape == s
		end, 44, drawn(function(l: Full): Full
			return { shape = s, color = l.color, face = "open", extra = "none" }
		end)))
	end
	for _, col in M.COLORS do
		table.insert(colors, swatch("color", col.id, function(): boolean
			return current().color == col.id
		end, 32, ui.div { w = 22, h = 22, min_w = 22, shrink = 0, rounded = "$radius.full", bg = ui.raw(col.hex, "the swatch is the colour itself") }))
	end
	for _, f in M.FACES do
		table.insert(faces, swatch("face", f, function(): boolean
			return current().face == f
		end, 44, drawn(function(l: Full): Full
			return { shape = l.shape, color = l.color, face = f, extra = "none" }
		end)))
	end
	for _, x in M.EXTRAS do
		table.insert(extras, swatch("extra", x, function(): boolean
			return current().extra == x
		end, 44, drawn(function(l: Full): Full
			return { shape = l.shape, color = l.color, face = l.face, extra = x }
		end)))
	end
	return ui.column {
		w_full = true,
		gap = "$space.3",
		ui.row {
			w_full = true,
			items = "center",
			ui.div { grow = 1, label("Shape") },
			-- (A picture of its own: only for a buddy that exists, whose
			-- folder keeps it.)
			if props.seed and props.seed ~= "new"
				then ui.row {
					gap = "$space.1",
					ui.button {
						label = "Upload a picture",
						h = 26,
						px = "$space.2",
						items = "center",
						rounded = "$radius.full",
						bg = "$hover",
						hover = { bg = "$pressed" },
						on_click = function()
							ui.call("kx.buddy.picture", props.seed):after(function(path: any)
								if type(path) == "string" then
									local cur = M.resolve(props.look:peek(), props.seed)
									local next: Look = { shape = cur.shape, color = cur.color, face = cur.face, extra = cur.extra, image = path }
									props.look:set(next)
									if props.on_change then
										props.on_change(next)
									end
								end
							end, function() end)
						end,
						ui.text { "Upload", literal = true, color = "$text.secondary", text_size = "$font.size.xs", weight = "$font.weight.medium" },
					},
					ui.button {
						label = "Reset the look",
						h = 26,
						px = "$space.2",
						items = "center",
						rounded = "$radius.full",
						bg = "$hover",
						hover = { bg = "$pressed" },
						on_click = function()
							props.look:set({})
							if props.on_change then
								props.on_change({})
							end
						end,
						ui.text { "Reset", literal = true, color = "$text.secondary", text_size = "$font.size.xs", weight = "$font.weight.medium" },
					},
				}
				else nil,
			ui.button {
				label = "Randomize the look",
				h = 26,
				px = "$space.2",
				gap = "$space.1",
				items = "center",
				rounded = "$radius.full",
				bg = "$hover",
				hover = { bg = "$pressed" },
				on_click = randomize,
				ui.text { "Randomize", literal = true, color = "$text.secondary", text_size = "$font.size.xs", weight = "$font.weight.medium" },
			},
		},
		ui.row(shapes),
		label("Colour"),
		ui.row(colors),
		label("Eyes"),
		ui.row(faces),
		label("Extra"),
		ui.row(extras),
	}
end)

return setmetatable(M, {
	__call = function(_, props: Props): Node
		return View(props)
	end,
})
