--!strict
-- Home: the Ray bar alone, centred, and a line of greeting above it. With
-- no project yet, one calm way in: choose a folder. Under the bar, three
-- first prompts for the project it will use, and what's under way.

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

local SUGGESTIONS = {
	{ icon = "magnifer", text = "Explain how this project is put together" },
	{ icon = "checklist", text = "Find and fix the flakiest test" },
	{ icon = "pen", text = "Tidy what changed since yesterday" },
}

local function suggestion(s: { icon: string, text: string }): Node
	return kx.Button {
		tone = "ghost",
		size = "sm",
		shape = "pill",
		on_click = function()
			ray.suggest(s.text)
		end,
		kx.Icon { name = s.icon, size = 14 },
		ui.text { s.text, literal = true },
	}
end

-- What's under way: the threads working or waiting, as a quiet line each.
local busy = ui.computed(function(): { store.Thread }
	local out = {}
	for _, t in store.threads:get() do
		if t.status == "running" or t.status == "needs_you" then
			table.insert(out, t)
		end
	end
	return out
end)

local function busy_row(t: store.Thread): Node
	local function get()
		return t
	end
	return kx.Row {
		size = "sm",
		on_click = function()
			nav.go("thread")
			store.open(get().id)
		end,
		kx.AgentMark {
			agent = get().harness_id,
			live = function()
				return get().status == "running"
			end,
			needs_you = function()
				return get().status == "needs_you"
			end,
		},
		ui.text {
			function()
				return get().title
			end,
			literal = true,
			grow = 1,
			truncate = true,
			color = "$text.secondary",
		},
		ui.text {
			function()
				local t = get()
				return if t.status == "needs_you" then "needs you" else t.harness
			end,
			literal = true,
			text_size = "$font.size.xs",
			color = function()
				return if get().status == "needs_you" then "$kx.attention" else "$text.tertiary"
			end,
		},
	}
end

local function no_project(): Node
	return ui.column {
		w_full = true,
		items = "center",
		gap = "$space.3",
		ui.column { color = "$accent", mb = "$space.2", kx.Icon { name = "orbit", size = 40 } },
		kx.Title { "Welcome to Keplorayx", literal = true, align = "center" },
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

function M.view(): Node
	local has_project = ui.computed(function()
		return #store.projects:get() > 0
	end)
	return ui.column {
		grow = 1,
		w_full = true,
		h_full = true,
		items = "center",
		justify = "center",
		px = "$space.6",
		pb = "$space.8",
		ui.column {
			w_full = true,
			max_w = 640,
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
						kx.Title { greeting(), literal = true, align = "center" },
						kx.Caption { "What should your agents work on?", literal = true, align = "center" },
					},
					ray.Ray { place = "home" },
					ui.row {
						w_full = true,
						flex_wrap = true,
						justify = "center",
						gap = "$space.1",
						suggestion(SUGGESTIONS[1]),
						suggestion(SUGGESTIONS[2]),
						suggestion(SUGGESTIONS[3]),
					},
					ui.show(function()
						return #busy:get() > 0
					end, ui.column {
						w_full = true,
						mt = "$space.4",
						gap = "$kx.space.hair",
						kx.Hint { "Under way", literal = true, px = "$space.2", pb = "$space.1" },
						ui.each(busy, busy_row, {
							key = function(t: store.Thread)
								return t.id
							end,
							w_full = true,
							gap = "$kx.space.hair",
						}),
					}),
				},
				no_project()
			),
		},
	}
end

return M
