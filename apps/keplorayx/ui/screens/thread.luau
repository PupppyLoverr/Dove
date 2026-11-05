--!strict
-- A thread: its title bar (what it's doing, the agent, the panel's tabs),
-- Zeron's transcript (ds/transcript, vendored; MIT, see
-- THIRD_PARTY_NOTICES.md) fed by screens/conversation, and the composer at
-- its foot, where what the agent asks (an approval, a question) opens too.

local kx = require("@kx")
local Status = require("@kx/status")
local Transcript = require("@ds/transcript").Transcript
local conversation = require("@screens/conversation")
local store = require("@kx/store")
local ray = require("@screens/ray")
local changes = require("@screens/changes")
local panel = require("@screens/panel")
local nav = require("@screens/nav")

local M = {}

-- ── The page ───────────────────────────────────────────────────────────

local function thread_kind(): Status.Kind
	local t = store.thread:get()
	if not t then
		return "idle"
	end
	if store.approval:get() then
		return "approval"
	end
	return if t.running then "working" else "done"
end

local function panel_button(tab: string, icon: string, label: string): Node
	return kx.Button {
		tone = "ghost",
		size = "sm",
		icon = true,
		label = label,
		tooltip = label,
		on_click = function()
			panel.toggle(tab)
		end,
		kx.Icon {
			name = icon,
			size = 16,
			color = function()
				return if changes.open:get() and panel.tab:get() == tab then "$text.primary" else "$text.tertiary"
			end,
		},
	}
end

local header = ui.row {
	h = "$kx.titlebar",
	w_full = true,
	window_drag = true,
	items = "center",
	px = "$space.4",
	gap = "$space.2",
	pr = function()
		return if changes.open:get() then "$space.3" else nav.caption_inset()
	end,
	ui.show(function()
		return not nav.sidebar_open:get()
	end, kx.Button {
		tone = "ghost",
		size = "sm",
		icon = true,
		label = "Show the sidebar (Ctrl+B)",
		tooltip = "Show the sidebar  Ctrl+B",
		on_click = function()
			nav.sidebar_open:set(true)
		end,
		kx.Icon { name = "sidebar", size = 16 },
	}),
	kx.StatusIcon { kind = thread_kind, size = 15 },
	ui.text {
		function()
			local t = store.thread:get()
			return if t then t.title else ""
		end,
		literal = true,
		truncate = true,
		min_w = 0,
		weight = "$font.weight.medium",
	},
	ui.text {
		function()
			local t = store.thread:get()
			if not t then
				return ""
			end
			return if t.running and t.status ~= "" then `{t.harness} · {t.status}` else t.harness
		end,
		literal = true,
		truncate = true,
		min_w = 0,
		shrink = 2,
		max_w = 320,
		text_size = "$font.size.sm",
		color = "$text.tertiary",
	},
	ui.div { grow = 1 },
	panel_button("files", "folder-with-files", "Files  Ctrl+P"),
	panel_button("browser", "global", "Browser  Ctrl+Shift+B"),
	panel_button("terminal", "terminal", "Terminal  Ctrl+`"),
	panel_button("changes", "file-diff", "Changes  Ctrl+J"),
}

-- (The panel beside it is the window's: app.luau.)
function M.view(): Node
	return M.column()
end

function M.column(): Node
	return ui.column {
		grow = 1,
		min_w = 0,
		h_full = true,
		-- It gives way to the panel beside it: what it holds wraps or
		-- truncates, never pushes the window wider.
		overflow = "hidden",
		header,
		-- Zeron's transcript: virtualized, following the end while a reply
		-- streams, its tools grouped between the reply's words.
		ui.column {
			grow = 1,
			min_h = 0,
			w_full = true,
			-- (A component of its own: the transcript keeps state of its own.)
			ui.component(function(): Node
				return Transcript {
					messages = conversation.messages,
					draft = conversation.draft,
					measure = kx.MEASURE,
				}
			end)({}),
		},
		ui.column {
			w_full = true,
			items = "center",
			px = "$space.6",
			pb = "$space.4",
			gap = "$space.2",
			ui.column {
				w_full = true,
				max_w = kx.MEASURE + 40,
				gap = "$space.2",
				-- (What the agent waits on opens in the composer itself.)
				ray.Ray { place = "thread" },
			},
		},
	}
end

return M
