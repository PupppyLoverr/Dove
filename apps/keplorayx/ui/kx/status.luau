--!strict
-- What an agent is doing, at a glance: a state's glyph in its hue (a row's
-- end, a header), the working ring that runs while it works, or a list
-- row's dot. Ported from Zeron's ds/status (MIT; see
-- THIRD_PARTY_NOTICES.md): its ring shader and its rule that a loop runs
-- only while it's seen, in Orbit's tokens and words.
--
--   StatusIcon { kind = "working" }            the ring, 16 points
--   StatusIcon { kind = function() … end }     follows a state in place
--   StatusDot { kind = "approval" }            9 points, pulsing if live
--   Spinner { size = 14, ink = "$kx.primary.text" }
--   Working { since = started }                ring, "Working", 42s

local motion = require("@kx/motion")
local Icon = require("@kx/icon").Icon

local M = {}

export type Kind = "working" | "input" | "approval" | "done" | "failed" | "idle"

local GLYPH: { [Kind]: string? } = {
	input = "status-needs-you",
	approval = "status-needs-you",
	done = "status-done",
	failed = "status-failed",
	idle = "status-idle",
}

M.LABEL = {
	working = "Working",
	input = "Has a question",
	approval = "Needs your approval",
	done = "Done",
	failed = "Stopped with an error",
	idle = "Idle",
} :: { [Kind]: string }

-- An engine status ("running", "needs_you", "done", "failed", "idle") as a
-- kind.
function M.kind_of(status: string?, needs: boolean?): Kind
	if needs or status == "needs_you" then
		return "approval"
	elseif status == "running" or status == "working" then
		return "working"
	elseif status == "done" or status == "finished" then
		return "done"
	elseif status == "failed" or status == "error" then
		return "failed"
	end
	return "idle"
end

-- How long it has worked: "42s", "5m", "1h 5m".
function M.elapsed(seconds: number): string
	local s = math.max(0, math.floor(seconds))
	if s < 60 then
		return `{s}s`
	elseif s < 3600 then
		return `{s // 60}m`
	end
	local m = (s % 3600) // 60
	return if m > 0 then `{s // 3600}h {m}m` else `{s // 3600}h`
end

-- The working ring: eight dashes on a 24-unit grid. An unseen lead runs the
-- ring twice in one eased sweep; the dashes near it stretch toward it and
-- let go as it passes, then settle to eight even dashes and rest. All on
-- the GPU from the time, at 30 frames a second, and nothing while it rests.
-- params[0]: its ink (a token, resolved by zgx in the first parameter).
local RING = ui.shader([[
fn kx_arc(d:vec2<f32>,R:f32,c:f32,ha:f32,hw:f32)->f32 {
 var th=degrees(atan2(d.x,-d.y));
 var da=th-c; da=da-360.*round(da/360.);
 if abs(da)<=ha { return abs(length(d)-R)-hw; }
 let e=radians(c+sign(da)*ha);
 return length(d-vec2(sin(e),-cos(e))*R)-hw;
}
fn fill(input:FillInput)->vec4<f32> {
 let ink=input.params[0];
 let u=min(input.size.x,input.size.y)/24.;
 let d=input.p-input.size*.5;
 let R=10.*u; let hw=u;
 let RUN=3.9;
 let ph=clamp(input.time,0.,RUN);
 let amp=smoothstep(0.,.55,ph)*(1.-smoothstep(RUN-.75,RUN,ph));
 let tau=clamp(ph/RUN,0.,1.);
 let lead=720.*(.5-.5*cos(3.14159265*tau));
 let speed=sin(3.14159265*tau);
 let pull=lead-14.*speed;
 var a=0.;
 var lit=0.;
 for (var i=0;i<8;i++) {
  let c=45.*f32(i);
  var dl=pull-c; dl=dl-360.*round(dl/360.);
  let sig=select(40.,26.,dl<0.);
  let g=amp*exp(-(dl*dl)/(sig*sig));
  let cov=clamp(.5-kx_arc(d,R,c+sign(dl)*9.*g,10.95*(1.+1.3*g),hw)*input.scale,0.,1.);
  a=max(a,cov);
  lit=max(lit,g*cov);
 }
 let t=.6*lit/max(a,.001);
 let toward=select(vec3(0.),vec3(1.),dot(ink.rgb,vec3(.2126,.7152,.0722))>.5);
 return vec4(mix(ink.rgb,toward,t),ink.a*a);
}
]])

local RUN, REST = 3.9, 900

local function ring(ink: string, size: number): Node
	return motion.looping({
		w = size,
		h = size,
		shrink = 0,
		bg_shader = RING,
		shader_params = { ink, { 0, 0, 0, 0 }, { 0, 0, 0, 0 }, { 0, 0, 0, 0 } },
	}, function(el)
		return anime.animate(el, {
			shaderTime = { 0, RUN },
			duration = RUN * 1000,
			loop = true,
			loopDelay = REST,
			ease = "linear",
			frameRate = motion.PULSE.fps,
		})
	end)
end

-- The ring alone, as a spinner (a busy button's, in its own text colour).
function M.Spinner(props: { size: number?, ink: string? }): Node
	return ring(props.ink or "$kx.status.working", props.size or 14)
end

local function glyph(kind: Kind, size: number): Node?
	if kind == "working" then
		return ring("$kx.status.working", size)
	end
	local name = GLYPH[kind]
	return if name then Icon { name = name, size = size } else nil
end

-- A state's glyph alone, in its hue. `kind` may be a function: the glyph
-- and hue follow it in place, and only a move between the ring and a glyph
-- swaps what's drawn.
function M.StatusIcon(props: { kind: Kind | () -> Kind, size: number? }): Node
	local size = props.size or 16
	local given = props.kind
	if type(given) ~= "function" then
		local kind = given :: Kind
		return ui.div { shrink = 0, color = `$kx.status.{kind}`, tooltip = M.LABEL[kind], glyph(kind, size) }
	end
	local kind = given :: () -> Kind
	return ui.div {
		shrink = 0,
		color = function(): string
			return `$kx.status.{kind()}`
		end,
		ui.switch(function(): string
			return if kind() == "working" then "ring" else "glyph"
		end, {
			ring = ring("$kx.status.working", size),
			glyph = Icon {
				name = function(): string
					return GLYPH[kind()] or "status-idle"
				end,
				size = size,
			},
		}),
	}
end

-- A list row's dot: its state's hue, its name a tooltip; live, it pulses.
function M.StatusDot(props: { kind: Kind, size: number?, color: string? }): Node
	local size = props.size or 8
	local dot = {
		w = size,
		h = size,
		shrink = 0,
		rounded = "$radius.full",
		bg = props.color or `$kx.status.{props.kind}`,
		tooltip = M.LABEL[props.kind],
		label = M.LABEL[props.kind],
	}
	if props.kind == "working" or props.kind == "approval" then
		return motion.pulse(dot, 0.45)
	end
	return ui.div(dot)
end

-- The ring, a word for it and how long: under a turn while the agent works.
-- `since`: when it began, in `os.time()` seconds (or a function of it).
M.Working = ui.component(function(props: { since: number | () -> number, label: string? }): Node
	local now = ui.use_state(os.time())
	ui.every(1000, function()
		now:set(os.time())
	end)
	local function since(): number
		local given: any = props.since
		return if type(given) == "function" then given() else given
	end
	return ui.row {
		h = 26,
		gap = "$space.2",
		items = "center",
		selectable = false,
		M.StatusIcon { kind = "working", size = 16 },
		ui.text { props.label or "Working", text_size = "$font.size.md", color = "$text.secondary" },
		ui.text {
			function(): string
				return M.elapsed(now:get() - since())
			end,
			tabular = true,
			text_size = "$font.size.md",
			color = "$text.tertiary",
		},
	}
end)

return M
