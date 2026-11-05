--!strict
-- Constellation: every thread at once, on a calm sky. Each project is a
-- centre; its threads orbit it, the busier ones larger and nearer. A body
-- breathes while its agent works and turns warm when it needs you. Click
-- one to dive back in. Ctrl+- (or Ctrl+Shift+O) zooms out to it.

local kx = require("@kx")
local store = require("@kx/store")
local nav = require("@screens/nav")

local M = {}

type Body = { thread: store.Thread, x: number, y: number, size: number }
type System = { name: string, x: number, y: number, bodies: { Body } }

local area = ui.ref()

-- Where everything sits: projects on a loose ring, threads on orbits round
-- them. Deterministic, so the sky is the same each time you look.
local systems = ui.computed(function(): { System }
	-- The sky's size, followed: it lays out again as the window resizes.
	local r: any = area
	local w = r.width or 900
	local h = r.height or 600
	local by: { [string]: System } = {}
	local order: { System } = {}
	for _, t in store.threads:get() do
		local s = by[t.project_id]
		if not s then
			s = { name = t.project, x = 0, y = 0, bodies = {} }
			by[t.project_id] = s
			table.insert(order, s)
		end
		table.insert(s.bodies, { thread = t, x = 0, y = 0, size = 10 })
	end
	local n = #order
	local cx, cy = w / 2, h / 2
	local ring = math.min(w, h) * (if n <= 1 then 0 else 0.3)
	for i, s in order do
		local a = (i - 1) / math.max(1, n) * 2 * math.pi - math.pi / 2
		s.x = cx + math.cos(a) * ring
		s.y = cy + math.sin(a) * ring
		for j, b in s.bodies do
			local t = b.thread
			local busy = t.status == "running" or t.status == "needs_you"
			local orbit = (if busy then 54 else 84) + (j % 3) * 12
			local angle = j * 2.39996 -- the golden angle: no two bodies line up
			b.x = s.x + math.cos(angle) * orbit
			b.y = s.y + math.sin(angle) * orbit
			b.size = if busy then 16 elseif t.unread then 12 else 9
		end
	end
	return order
end)

local function body_view(b: Body): Node
	local t = b.thread
	local needs = t.status == "needs_you"
	local live = t.status == "running"
	local glow = if live or needs then kx.motion.breath(0.45, 1) else nil
	return ui.button {
		absolute = true,
		left = ui.raw(b.x - 14, "a body's place on the sky"),
		top = ui.raw(b.y - 14, "a body's place on the sky"),
		w = 28,
		h = 28,
		p = 0,
		rounded = "$radius.full",
		bg = "#00000000",
		hover = { bg = "$hover" },
		items = "center",
		justify = "center",
		cursor = "pointer",
		label = t.title,
		tooltip = `{t.title} · {if needs then "needs you" else t.harness}`,
		on_click = function()
			nav.go("thread")
			store.open(t.id)
		end,
		ui.div {
			w = b.size,
			h = b.size,
			rounded = "$radius.full",
			bg = if needs then "$kx.attention" else kx.tokens.agent(t.harness_id),
			opacity = if glow then glow else 0.75,
			shadow = if needs then "$kx.glow.attention" elseif live then "$kx.glow.live" else nil,
		},
	}
end

local function system_view(s: System): Node
	local bodies: { Node } = {}
	for _, b in s.bodies do
		table.insert(bodies, body_view(b))
	end
	return ui.div {
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		pointer_events = false,
		-- The project at the centre: a faint core and its name.
		ui.div {
			absolute = true,
			left = ui.raw(s.x - 3, "the system's centre"),
			top = ui.raw(s.y - 3, "the system's centre"),
			w = 6,
			h = 6,
			rounded = "$radius.full",
			bg = "$text.tertiary",
		},
		ui.text {
			s.name,
			literal = true,
			absolute = true,
			left = ui.raw(s.x - 80, "the system's name, centred under it"),
			top = ui.raw(s.y + 10, "the system's name, centred under it"),
			w = 160,
			align = "center",
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		},
		ui.div {
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			bottom = 0,
			pointer_events = true,
			table.unpack(bodies),
		},
	}
end

function M.view(): Node
	return ui.column {
		grow = 1,
		min_w = 0,
		h_full = true,
		ui.row {
			h = "$kx.titlebar",
			w_full = true,
			window_drag = true,
			items = "center",
			pl = nav.bar_left,
			pr = nav.caption_inset,
			gap = "$space.3",
			kx.Label { "Constellation", literal = true },
			kx.Hint { "every thread at once", literal = true },
		},
		ui.div {
			ref = area,
			grow = 1,
			w_full = true,
			min_h = 0,
			relative = true,
			overflow = "hidden",
			radial = function()
				return { ui.theme.get("kx.sky.mid"), ui.theme.get("kx.sky.deep") }
			end,
			ui.show(
				function()
					return #store.threads:get() > 0
				end,
				ui.each(systems, system_view, {
					key = function(s: System)
						return s.name
					end,
					absolute = true,
					top = 0,
					left = 0,
					right = 0,
					bottom = 0,
				}),
				ui.column {
					absolute = true,
					top = 0,
					left = 0,
					right = 0,
					bottom = 0,
					items = "center",
					justify = "center",
					kx.Empty { icon = "constellation", title = "A clear sky", hint = "Start a thread and its agent appears here." },
				}
			),
		},
	}
end

return M
