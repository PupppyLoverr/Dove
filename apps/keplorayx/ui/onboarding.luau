--!strict
-- Setup, First Light's second half: one calm card a step, progress as
-- small planets along an arc. Every step can be skipped and reached again
-- from Settings; `KX_STEP=<step>` opens on one and keeps it across saves.
--
-- The intro (intro.luau) opens this window hidden under its sky and hands
-- over on one frame: until `onboarding.arrived`, the frame holds solid so
-- both copies match; then it clears to its glass.

ui.theme.strict("warn")
local kx = require("@kx")
kx.setup()

local accent = require("@kx/accent")
local steps = require("@screens/onboarding/steps")

local arrived = ui.shared("onboarding.arrived", true)
local dev_step = (ui.shared("onboarding.step", "") :: any):peek() :: string

local ORDER = steps.ORDER
local at = ui.persist("onboarding.at", 1)
if dev_step ~= "" then
	for i, s in ORDER do
		if s.id == dev_step then
			at:set(i)
		end
	end
end
-- Which way the last move went, for the card's slide.
local forward = ui.state("onboarding.forward", true)

local function go(i: number)
	forward:set(i >= at:peek())
	at:set(math.clamp(i, 1, #ORDER))
end

local function finish()
	ui.invoke("onboarding.finish")
end

local function next()
	if at:peek() >= #ORDER then
		finish()
	else
		go(at:peek() + 1)
	end
end

-- The planets: one a step along a shallow arc, the one you're on lit and
-- ringed, the ones behind you filled, the ones ahead faint.
local function planets(): Node
	local dots: { Node } = {}
	for i, s in ORDER do
		local lift = math.sin((i - 1) / (#ORDER - 1) * math.pi) * 10
		table.insert(dots, ui.button {
			w = 22,
			h = 22,
			p = 0,
			bg = "#00000000",
			hover = { bg = "#00000000" },
			items = "center",
			justify = "center",
			cursor = "pointer",
			label = s.title,
			tooltip = s.title,
			mt = ui.raw(10 - lift, "the arc's rise, computed per planet"),
			on_click = function()
				go(i)
			end,
			ui.div {
				w = function()
					return if at:get() == i then 10 else 6
				end,
				h = function()
					return if at:get() == i then 10 else 6
				end,
				rounded = "$radius.full",
				bg = function()
					return if at:get() >= i then "$accent" else "$border.strong"
				end,
				outline = function()
					return if at:get() == i then 2 else 0
				end,
				outline_color = "$accent",
			},
		})
	end
	return ui.row { gap = "$space.3", items = "start", h = 36, table.unpack(dots) }
end

local function step_view(i: number): Node
	return ORDER[i].view()
end

local card = ui.column {
	w = 560,
	h = 420,
	gap = "$space.4",
	ui.presence(function()
		return tostring(at:get())
	end, function(key: string)
		return step_view(tonumber(key) or 1)
	end, {
		grow = 1,
		w_full = true,
		min_h = 0,
		mode = "wait",
		enter = {
			opacity = { 0, 1 },
			x = function()
				return if forward:peek() then { 28, 0 } else { -28, 0 }
			end,
			duration = 300,
			ease = kx.motion.ENTER,
		},
		exit = { opacity = { 1, 0 }, duration = 120 },
	}),
	ui.row {
		w_full = true,
		items = "center",
		gap = "$space.2",
		ui.show(function()
			return at:get() > 1
		end, kx.Button {
			tone = "ghost",
			on_click = function()
				go(at:peek() - 1)
			end,
			"Back",
		}),
		ui.div { grow = 1 },
		ui.show(function()
			local s = ORDER[at:get()]
			return s.optional == true
		end, kx.Button { tone = "ghost", on_click = next, "Skip" }),
		kx.Button {
			size = "lg",
			on_click = next,
			ui.text {
				function()
					local s = ORDER[at:get()]
					return s.next or "Continue"
				end,
				literal = true,
			},
			kx.Icon { name = "arrow-right", size = 16 },
		},
	},
}

return ui.column {
	w_full = true,
	h_full = true,
	items = "center",
	theme = accent.scope(),
	-- Solid until the intro hands over, so its copy and this one match.
	bg = function()
		return if arrived:get() then kx.material.window_bg()() else "$kx.solid"
	end,
	on_key = function(e: any)
		if e.editing then
			return nil
		end
		if e.key == "ArrowRight" then
			next()
			return true
		elseif e.key == "ArrowLeft" and at:peek() > 1 then
			go(at:peek() - 1)
			return true
		end
		return nil
	end,
	ui.row { h = "$kx.titlebar", w_full = true, window_drag = true },
	planets(),
	ui.column { grow = 1, w_full = true, items = "center", justify = "center", pb = "$space.6", card },
	kx.Toasts {},
}
