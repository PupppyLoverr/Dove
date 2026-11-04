--!strict
-- The panel beside a thread: Changes or Terminal, one at a time, under one
-- header. Ctrl+J opens it on Changes, Ctrl+` on Terminal; the same key
-- again closes it.

local kx = require("@kx")
local nav = require("@screens/nav")
local changes = require("@screens/changes")
local terminal = require("@screens/terminal")

local M = {}

-- Open or not lives with Changes (its refresh listens), the tab here.
M.open = changes.open
local store = require("@kx/store")
M.tab = if store.fixture ~= "" then ui.state("panel.tab", "changes") else ui.persist("panel.tab", "changes")

-- Development: a fixture may ask for a tab.
local fx = (store :: any).fixture_data
if fx and fx.panel_tab then
	M.tab:set(fx.panel_tab)
	M.open:set(true)
end

function M.toggle(tab: string)
	if M.open:peek() and M.tab:peek() == tab then
		M.open:set(false)
	else
		M.tab:set(tab)
		M.open:set(true)
	end
end

function M.view(): Node
	return ui.column {
		w = 460,
		shrink = 0,
		h_full = true,
		bg = function()
			return if kx.material.see_through() then "$surface.sunken" else "$kx.solid"
		end,
		border_sides = { 0, 0, 0, 1 },
		border_color = "$border.subtle",
		ui.row {
			h = "$kx.titlebar",
			w_full = true,
			pl = "$space.3",
			pr = nav.caption_inset,
			gap = "$space.2",
			items = "center",
			window_drag = true,
			kx.Segmented {
				value = M.tab,
				label = "Panel",
				width = 84,
				options = { { label = "Changes", value = "changes" }, { label = "Terminal", value = "terminal" } },
			},
			ui.div { grow = 1 },
			ui.show(function()
				return M.tab:get() == "changes"
			end, changes.summary()),
		},
		ui.switch(M.tab, {
			changes = ui.column { grow = 1, min_h = 0, w_full = true, changes.list() },
			terminal = ui.column { grow = 1, min_h = 0, w_full = true, terminal.view() },
		}),
	}
end

return M
