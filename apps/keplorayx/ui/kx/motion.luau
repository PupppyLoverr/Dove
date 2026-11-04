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
M.EASE = "cubicBezier(0.22, 1.12, 0.36, 1)"
M.ENTER = "cubicBezier(0.16, 1.18, 0.3, 1)"
M.EXIT = "cubicBezier(0.4, 0, 0.8, 0.4)"
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

-- ── The breath ─────────────────────────────────────────────────────────
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
