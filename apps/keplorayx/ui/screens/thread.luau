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

local width = ui.persist("kx.conversation_width", "default")

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

-- The thread's head, as Zeron's (its screens/shell/page.luau): on the title
-- bar's line, always, its agent's mark (or its state while it works or
-- waits) and its title, small and quiet; past the title bar's controls when
-- the sidebar is folded away. Room to move the window by.
local TitleBar = require("@ds/title_bar")
local function agent_id(): string
	local t = store.thread:get()
	if not t then
		return ""
	end
	for _, th in store.threads:get() do
		if th.id == t.id then
			return th.harness_id
		end
	end
	return ""
end
local header = ui.row {
	id = "session-header",
	h = 40,
	w_full = true,
	shrink = 0,
	window_drag = true,
	items = "center",
	gap = "$space.2",
	pl = nav.bar_left,
	pr = ui.raw(150, "the title bar's controls at the right"),
	ui.presence(function(): string
		local k = thread_kind()
		return if k == "working" or k == "approval" then k else "mark:" .. agent_id()
	end, function(key: string): Node
		local agent = string.match(key, "^mark:(.*)$")
		if agent then
			return if agent ~= "" then kx.AgentMark { agent = agent, size = 14 } else ui.div {}
		end
		return kx.StatusIcon { kind = key :: any, size = 14 }
	end),
	ui.text {
		id = "page-title",
		function()
			local t = store.thread:get()
			return if t then t.title else ""
		end,
		literal = true,
		truncate = true,
		min_w = 0,
		grow = 1,
		text_size = "$font.size.sm",
		weight = "$font.weight.medium",
		color = "$text.secondary",
	},
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
					-- Settings › Appearance › Conversation width.
					measure = function(): number
						local w = width:get()
						return if w == "narrow" then 600 elseif w == "wide" then 860 else kx.MEASURE
					end,
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
