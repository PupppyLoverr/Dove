-- The conversations: a new chat, a search, and the list, newest first.
-- Each one opens on a click (or the arrows); its context menu deletes it.
local state = require("../state")
local api = require("../api")

type Conversation = state.Conversation

-- The conversations the search finds.
local found = ui.computed(function(): { Conversation }
	local query = state.query:get():lower()
	local all = state.conversations:get()
	if query == "" then
		return all
	end
	local out: { Conversation } = {}
	for _, c in all do
		if c.title:lower():find(query, 1, true) then
			table.insert(out, c)
		end
	end
	return out
end)

local function remove(id: string)
	if state.streaming:peek() == id then
		api.stop()
	end
	local at = state.conversations:index_of("id", id)
	if not at then
		return
	end
	ui.batch(function()
		state.conversations:remove(at)
		state.messages:set_key(id, nil)
		if state.current:peek() == id then
			local next = state.conversations:peek()[1]
			if next then
				state.current:set(next.id)
			else
				api.new_conversation()
			end
		end
	end)
end

local function row(c: Conversation): Node
	return ui.button {
		id = `conversation-{c.id}`,
		role = "option",
		selected = function()
			return state.current:get() == c.id
		end,
		w_full = true,
		px = 10,
		py = 7,
		rounded = 8,
		justify = "start",
		bg = function()
			return if state.current:get() == c.id then "$selection.item" else "#00000000"
		end,
		hover = { bg = "$hover" },
		on_click = function()
			state.current:set(c.id)
		end,
		context_menu = function()
			return {
				ui.menu_item {
					"Delete",
					on_click = function()
						remove(c.id)
					end,
				},
			}
		end,
		ui.row {
			w_full = true,
			gap = 6,
			items = "center",
			ui.text { c.title, literal = true, truncate = true, grow = 1, min_w = 0 },
			ui.show(function()
				return state.streaming:get() == c.id
			end, ui.text { "●", color = "$accent", text_size = 8, aria_hidden = true }),
		},
	}
end

local M = {}

M.remove = remove

function M.view(): Node
	return ui.column {
		w = 260,
		h_full = true,
		shrink = 0,
		pt = 40,
		px = 10,
		gap = 8,
		bg = "$surface.sunken",
		ui.button {
			id = "new",
			"New chat",
			tooltip = "⌘N",
			w_full = true,
			on_click = api.new_conversation,
		},
		ui.input {
			id = "search",
			ref = state.search,
			value = state.query,
			label = "Search conversations",
			placeholder = "Search",
			w_full = true,
		},
		ui.scroll {
			role = "listbox",
			label = "Conversations",
			w_full = true,
			grow = 1,
			gap = 2,
			pb = 10,
			ui.each(found, row, {
				key = function(c: Conversation): string
					return c.id
				end,
			}),
		},
	}
end

return M
