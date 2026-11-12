--!strict
-- A buddy's face: one filled shape with two eyes, in its colour. The idea is
-- bloub's (https://github.com/jeremy-prt/bloub, MIT: a body shape, a colour
-- and a rest expression to choose from); drawn here on a canvas, with our own
-- shapes and measures.
--
--   Avatar { look = { shape, color, face }, seed = buddy.id, size = 28,
--            standing = function(): string? … end, live = true }
--
-- `look` may leave any part out: it's chosen from `seed`, so a buddy with no
-- look of its own still has one that stays the same. `standing` changes the
-- eyes while it works (they look aside), waits on you (they look up) or has
-- failed (they close). `live`: it blinks now and then.

export type Look = { shape: string?, color: string?, face: string? }
export type Props = {
	look: Look? | (() -> Look?),
	seed: string?,
	size: number?,
	standing: (() -> string?)?,
	live: boolean?,
}

local M = {}

M.SHAPES = { "circle", "squircle", "soft", "tall", "wide", "stone" }
M.FACES = { "open", "calm", "round", "sleepy", "wink", "up" }
-- (Each with whether it's a light one: its eyes are dark then.)
M.COLORS = {
	{ id = "ink", hex = "#16171c", light = false },
	{ id = "snow", hex = "#f2f3f5", light = true },
	{ id = "blue", hex = "#4d8dff", light = false },
	{ id = "sky", hex = "#7fd0ff", light = true },
	{ id = "mint", hex = "#5fd3a2", light = true },
	{ id = "green", hex = "#2f9e62", light = false },
	{ id = "sun", hex = "#ffcf4d", light = true },
	{ id = "amber", hex = "#ff9a4d", light = true },
	{ id = "rose", hex = "#ff6b8e", light = false },
	{ id = "red", hex = "#e5484d", light = false },
	{ id = "violet", hex = "#9a7bff", light = false },
	{ id = "plum", hex = "#6b3fa0", light = false },
}

local function hash(s: string): number
	local h = 7
	for i = 1, #s do
		h = (h * 31 + string.byte(s, i)) % 2147483647
	end
	return h
end

-- A look with every part said: what's given, else what its seed chooses.
function M.resolve(look: Look?, seed: string?): { shape: string, color: string, face: string }
	local h = hash(seed or "buddy")
	local l: Look = look or {}
	return {
		shape = l.shape or M.SHAPES[h % #M.SHAPES + 1],
		-- (Never the two plain ones by chance: ink and snow are chosen.)
		color = l.color or M.COLORS[(h // 7) % (#M.COLORS - 2) + 3].id,
		face = l.face or M.FACES[(h // 91) % 3 + 1],
	}
end

local function color_of(id: string): (string, boolean)
	for _, c in M.COLORS do
		if c.id == id then
			return c.hex, c.light
		end
	end
	return M.COLORS[3].hex, false
end

-- The body: `w` by `h` about the middle, its corner as a share of the short side.
local BODY: { [string]: { w: number, h: number, corner: number } } = {
	circle = { w = 1, h = 1, corner = 0.5 },
	squircle = { w = 1, h = 1, corner = 0.32 },
	soft = { w = 1, h = 1, corner = 0.42 },
	tall = { w = 0.8, h = 1, corner = 0.5 },
	wide = { w = 1, h = 0.8, corner = 0.5 },
	stone = { w = 1, h = 0.92, corner = 0.24 },
}

local View = ui.component(function(props: Props): Node
		local size = props.size or 28
		-- (Its blink: shut for a moment every few seconds, each on its own beat.)
		local shut = ui.use_state(false)
		if props.live then
			local beat = 3400 + hash(props.seed or "") % 2600
			local stop = ui.every(beat, function()
				shut:set(true)
				ui.after(130, function()
					shut:set(false)
				end)
			end)
			ui.on_cleanup(stop)
		end
		return ui.canvas {
			w = size,
			h = size,
			shrink = 0,
			draw = function(c, w: number, h: number)
				local given = props.look
				local look = M.resolve(if type(given) == "function" then given() else given, props.seed)
				local hex, light = color_of(look.color)
				local body = BODY[look.shape] or BODY.circle
				local bw, bh = w * body.w, h * body.h
				local x, y = (w - bw) / 2, (h - bh) / 2
				c:rect(x, y, bw, bh, hex, math.min(bw, bh) * body.corner)
				-- The eyes: two capsules a little above the middle, a little
				-- toward one side, as if it looked past you.
				local standing = if props.standing then props.standing() else nil
				local face = look.face
				local ink = if light then "#0f1014" else "#f9f9f9"
				local ew, eh = bw * 0.11, bh * 0.26
				if face == "calm" then
					eh = bh * 0.17
				elseif face == "round" then
					ew, eh = bw * 0.15, bw * 0.15
				elseif face == "sleepy" then
					eh = bh * 0.06
				end
				local cx, cy = w / 2 + bw * 0.05, h / 2 - bh * 0.1
				local gap = bw * 0.2
				if standing == "working" or standing == "connecting" then
					cx += bw * 0.09
				elseif standing == "input" or standing == "approval" then
					cy -= bh * 0.09
				elseif face == "up" then
					cy -= bh * 0.07
				end
				local closed = shut:get() or standing == "failed"
				for i, side in { -1, 1 } do
					local height = eh
					if closed or (face == "wink" and i == 2) then
						height = math.max(1.5, bh * 0.05)
					end
					c:rect(cx + side * gap / 2 - ew / 2, cy - height / 2, ew, height, ink, math.min(ew, height) / 2)
				end
			end,
		}
end)

return setmetatable(M, {
	__call = function(_, props: Props): Node
		return View(props)
	end,
})
