-- Chat: a model's replies streaming in over server-sent events, drawn as
-- markdown (code blocks with Copy) in a virtualized transcript; a
-- composer that grows as you type; a sidebar of conversations with a
-- search; and the keyboard: ⌘N new chat, ⌘K search, ⌘L the composer,
-- Esc stops a reply, ⌘⇧⌫ deletes the conversation, ⌥↑/⌥↓ move between
-- conversations.
local state = require("./state")
local api = require("./api")
local Sidebar = require("@components/sidebar")
local Transcript = require("@components/transcript")
local Composer = require("@components/composer")

ui.theme.follow_system { dark = "zgx-dark", light = "zgx-light" }

-- The conversation `by` places down the sidebar from the one shown.
local function step(by: number)
	local all = state.conversations:peek()
	local at = state.conversations:index_of("id", state.current:peek()) or 1
	local next = all[math.clamp(at + by, 1, #all)]
	if next then
		state.current:set(next.id)
	end
end

local function focus(ref: Ref)
	local el = ref:peek()
	if el then
		el:focus()
	end
end

ui.command {
	id = "new",
	title = "New Chat",
	menu = "File",
	key = "cmd+n",
	run = function()
		api.new_conversation()
		focus(state.composer)
	end,
}
ui.command {
	id = "search",
	title = "Find Conversation",
	menu = "Edit",
	key = "cmd+k",
	run = function()
		focus(state.search)
	end,
}
ui.command {
	id = "compose",
	title = "Go to Message",
	menu = "Edit",
	key = "cmd+l",
	run = function()
		focus(state.composer)
	end,
}
ui.command { id = "stop", title = "Stop Reply", menu = "Chat", key = "escape", run = api.stop }
ui.command {
	id = "delete",
	title = "Delete Chat",
	menu = "Chat",
	key = "cmd+shift+backspace",
	run = function()
		Sidebar.remove(state.current:peek())
	end,
}
ui.command {
	id = "previous",
	title = "Previous Chat",
	menu = "Chat",
	key = "alt+up",
	run = function()
		step(-1)
	end,
}
ui.command {
	id = "next",
	title = "Next Chat",
	menu = "Chat",
	key = "alt+down",
	run = function()
		step(1)
	end,
}

return ui.row {
	w_full = true,
	h_full = true,
	color = "$text.primary",
	bg = "$surface.base",
	Sidebar.view(),
	ui.div { w = 1, h_full = true, shrink = 0, bg = "$border.subtle" },
	ui.column {
		grow = 1,
		min_w = 0,
		h_full = true,
		ui.row {
			w_full = true,
			h = 40,
			shrink = 0,
			px = 24,
			items = "center",
			window_drag = true,
			ui.text {
				function()
					local at = state.conversations:index_of("id", state.current:get())
					return if at then state.conversations:get()[at].title else ""
				end,
				id = "title",
				bold = true,
				truncate = true,
			},
		},
		Transcript(),
		Composer(),
	},
}
