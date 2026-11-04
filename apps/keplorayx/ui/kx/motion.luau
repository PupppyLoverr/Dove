--!strict
-- Orbit's one motion language. Every movement eases in, overshoots by a
-- breath and settles (`motion.ease.*` in the themes), and every live
-- indicator reads one shared clock, the breath, so nothing in the window
-- pulses out of step with anything else.
--
-- One place asks whether motion is reduced: the system's setting or the
-- app's own (`motion.reduce`, Settings). Reduced, a move lands at once and a
-- fade is a short crossfade; the breath stands still.
--
-- Durations grow with distance by its square root, to a cap, so a wide
-- panel travels at a narrow one's pace (a mechanism learnt from Zeron's
-- zgx app, MIT; see THIRD_PARTY_NOTICES.md). A moving column keeps its
-- content at full width and its edge clips it: nothing lays out mid-move.
--
--   local motion = require("@kx/motion")
--   w = motion.tween(width_state, motion.move(240))
--   opacity = motion.breath(0.55, 1)       -- a live indicator's glow

local prefers = require("@kx/prefers")

local M = {}

M.reduce = ui.persist("motion_reduce", false)

-- Tracked: in a binding or computed, it follows either setting.
function M.reduced(): boolean
	return M.reduce:get() or prefers.reduced_motion:get()
end

-- Untracked: where a move starts (a handler, an effect).
function M.reduced_now(): boolean
	return M.reduce:peek() or prefers.reduced_motion:peek()
end

-- The longest a reduced fade runs.
M.FADE = 120

-- The curves, by name: the themes say what each is.
M.EASE = "cubicBezier(0.2, 0, 0, 1)"
M.ENTER = "cubicBezier(0.16, 1, 0.3, 1)"
M.EXIT = "cubicBezier(0.4, 0, 1, 1)"
-- A fade has no overshoot to show: plain ease-out.
M.FADE_EASE = "out(3)"

local function ease(name: string): string
	local v = ui.theme.get(`motion.ease.{name}`)
	return if type(v) == "string" then v else M.EASE
end

local function ms(name: string, fallback: number): number
	local v = ui.theme.get(`motion.duration.{name}`)
	return if type(v) == "number" then v else fallback
end

-- A movement of `distance` points: its duration, by the square root of how
-- far it goes, from the fast step to the slow one.
function M.duration(distance: number): number
	if M.reduced_now() then
		return 0
	end
	local fast, slow = ms("fast", 140), ms("slow", 360)
	return math.clamp(fast + math.sqrt(math.max(0, distance)) * 9, fast, slow)
end

export type Tween = { duration: number, ease: string }

-- The options for a move of `distance` points.
function M.move(distance: number): Tween
	return { duration = M.duration(distance), ease = ease("standard") }
end

-- The options for something arriving (a panel settling, a row sliding in).
function M.enter(): Tween
	return { duration = if M.reduced_now() then M.FADE else ms("normal", 220), ease = ease("enter") }
end

-- The options for something leaving: quicker, no overshoot.
function M.exit(): Tween
	return { duration = if M.reduced_now() then M.FADE else ms("fast", 140), ease = ease("exit") }
end

-- The options for a fade.
function M.fade(): Tween
	return { duration = if M.reduced_now() then M.FADE else ms("normal", 220), ease = M.FADE_EASE }
end

-- A value that eases to where `target` goes, or (reduced) lands at once.
function M.tween(target: any, opts: Tween): any
	return if M.reduced_now() then target else ui.tween(target, opts)
end

function M.spring(target: any, opts: any): any
	return if M.reduced_now() then target else ui.spring(target, opts)
end

-- ── Loops that run only while seen ─────────────────────────────────────
-- A node whose loop (`start(el)`'s animation) plays only while it can be
-- seen, and never with motion reduced; out of sight (a collapsed sidebar,
-- a closed panel) it stops. The animation runs in the engine, not through
-- a script state, so a pulse redraws the one element and nothing else
-- (Zeron's `looping`, MIT; see THIRD_PARTY_NOTICES.md). The loop is held
-- weakly: held strongly it would hold its element, the element this node,
-- a cycle no collector sees.
function M.looping(props: any, start: (el: Element) -> any, make: ((any) -> Node)?): Node
	local held = setmetatable({}, { __mode = "v" }) :: { anim: any }
	props.on_visible = function(visible: boolean, el: Element)
		if visible then
			if not held.anim and not M.reduced_now() then
				held.anim = start(el)
			end
		elseif held.anim then
			held.anim:cancel()
			held.anim = nil
		end
	end
	return (make or ui.div)(props)
end

-- Every pulse's pace, so the frames that draw one draw them all: dimming
-- and brightening in six steps each way at 30 frames a second, each held
-- for a rest that draws nothing.
M.PULSE = { duration = 220, rest = 900, fps = 30 }

-- `props` (a container's) dimming to `low` and back while it's seen.
function M.pulse(props: any, low: number?, make: ((any) -> Node)?): Node
	return M.looping(props, function(el)
		return anime.animate(el, {
			opacity = { 1, low or 0.5 },
			duration = M.PULSE.duration,
			ease = "steps(6)",
			loop = true,
			alternate = true,
			loopDelay = M.PULSE.rest,
			frameRate = M.PULSE.fps,
		})
	end, make)
end

-- ── The breath ─────────────────────────────────────────────────────────
-- (Kept for the intro and Constellation's sky, which draw every frame
-- anyway; everything else pulses with `pulse`, which costs no script work.)
-- One clock for every live indicator, so they all glow in step. It runs
-- only while an indicator is mounted (each holds it from its component and
-- lets go as it unmounts), at 30 frames a second, and stands still with
-- motion reduced: an idle window, or one with nothing live, draws nothing.

local BREATH_MS = 2800
local phase = ui.state("kx.breath", 0)
local holders = 0
local timer: any = nil

local function hold()
	holders += 1
	if timer == nil then
		timer = anime.createTimer {
			loop = true,
			duration = BREATH_MS,
			frameRate = 30,
			onUpdate = function(a: any)
				-- A sine through the cycle: in and out, no seam.
				local t = (a.iterationCurrentTime or 0) / BREATH_MS
				phase:set(0.5 - 0.5 * math.cos(t * 2 * math.pi))
			end,
		}
	end
end

local function release()
	holders = math.max(0, holders - 1)
	if holders == 0 and timer ~= nil then
		timer:cancel()
		timer = nil
	end
end

-- Between `low` and `high`, in step with every other live thing. Call it
-- while a component builds (it holds the clock for that component's life).
function M.breath(low: number, high: number): () -> number
	hold()
	ui.on_cleanup(release)
	return function()
		if M.reduced() then
			return high
		end
		return low + (high - low) * phase:get()
	end
end

return M
