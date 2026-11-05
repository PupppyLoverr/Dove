--!strict
-- Home: a new thread, as Zeron's new chat sits (screens/chat/home; MIT, see
-- THIRD_PARTY_NOTICES.md): a night sky, a greeting set large in the
-- serif, and the composer alone under it. With no project yet, one calm way
-- in: choose a folder.

local kx = require("@kx")
local store = require("@kx/store")
local ray = require("@screens/ray")
local nav = require("@screens/nav")

local M = {}

local function greeting(): string
	local hour = tonumber(os.date("%H")) or 12
	if store.fixture ~= "" then
		hour = 20
	end
	if hour < 5 then
		return "Still up"
	elseif hour < 12 then
		return "Good morning"
	elseif hour < 18 then
		return "Good afternoon"
	end
	return "Good evening"
end

local function no_project(): Node
	return ui.column {
		w_full = true,
		items = "center",
		gap = "$space.3",
		ui.column { color = "$text.primary", mb = "$space.2", kx.Icon { name = "orbit", size = 36 } },
		ui.text {
			"Welcome to Keplorayx",
			literal = true,
			align = "center",
			font = "$kx.display",
			text_size = "$kx.hero.size",
			line_height = "$kx.hero.line",
			letter_spacing = "$kx.hero.tracking",
		},
		kx.Body {
			"Choose a folder to work in. Your agents run there, on this device; nothing leaves it unless you connect something that does.",
			literal = true,
			align = "center",
			max_w = 460,
			color = "$text.secondary",
		},
		ui.row {
			mt = "$space.3",
			gap = "$space.2",
			kx.Button {
				size = "lg",
				on_click = ray.pick_folder,
				kx.Icon { name = "folder-add", size = 18 },
				"Choose a folder",
			},
		},
		kx.Hint { "Or drop one onto this window", literal = true, mt = "$space.1" },
	}
end

-- The sky behind a new thread (Zeron's new-chat wallpaper, ds/wallpaper):
-- one of a few night skies under an effect, faded from the top to nothing
-- at its foot. Settings › Appearance chooses both; "none" is the bare glass.
M.SKIES = {
	{ value = "violet-dusk", label = "Dusk" },
	{ value = "aurora", label = "Aurora" },
	{ value = "saturnine", label = "Saturn" },
	{ value = "black-hole", label = "Event horizon" },
	{ value = "none", label = "None" },
}
M.sky = ui.persist("kx.sky", "violet-dusk")
M.sky_effect = ui.persist("kx.sky_effect", "dither")

local function sky(): Node
	local wallpaper = require("@ds/wallpaper")
	return ui.column {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		pointer_events = "none",
		wallpaper.Wallpaper {
			effect = M.sky_effect,
			-- Laid the window's width from the right, so it holds still as
			-- the sidebar comes and goes, and always reaches the page's left.
			width = function(): number
				local ok, size = pcall(function(): any
					return (ui :: any).window.size:get()
				end)
				return if ok and type(size) == "table" and size.width then size.width else 1600
			end,
			key = function(): string
				local s = M.sky:get()
				return if s == "none" then "" else s
			end,
			content = function(key: string): Node
				return wallpaper.Picture { src = `assets/cards/{key}/sky.webp`, w = 576, h = 768, y = 0.35 }
			end,
		},
	}
end

function M.view(): Node
	local has_project = ui.computed(function()
		return #store.projects:get() > 0
	end)
	return ui.column {
		grow = 1,
		w_full = true,
		h_full = true,
		relative = true,
		items = "center",
		justify = "center",
		sky(),
		px = "$space.6",
		pb = "$space.8",
		ui.column {
			w_full = true,
			max_w = 720,
			gap = "$space.5",
			ui.show(
				has_project,
				ui.column {
					w_full = true,
					gap = "$space.5",
					ui.column {
						w_full = true,
						gap = "$space.1",
						items = "center",
						ui.text {
							greeting(),
							literal = true,
							align = "center",
							font = "$kx.display",
							text_size = "$kx.hero.size",
							line_height = "$kx.hero.line",
							letter_spacing = "$kx.hero.tracking",
						},
					},
					ray.Ray { place = "home" },
					-- (Nothing else: what's under way is the sidebar's to say.)
				},
				no_project()
			),
		},
	}
end

return M
