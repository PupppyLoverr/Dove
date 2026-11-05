-- The app's state: every module that requires this one gets these States.
export type Role = "user" | "assistant"
export type Message = {
	id: string,
	role: Role,
	text: string,
	-- An assistant's reply while it streams in.
	streaming: boolean?,
	-- Why a reply stopped short ("Stopped", the server's error).
	error: string?,
}
export type Conversation = { id: string, title: string, updated: number }

local M = {}

local welcome: { Message } = {
	{ id = "m1", role = "user", text = "What can you do?" },
	{
		id = "m2",
		role = "assistant",
		text = "I answer as I think: my reply **streams in**, a few words at a time.\n\n"
			.. "- Ask for code and I write a block you can copy.\n"
			.. "- Press **Esc** to stop me, **⌘N** for a new chat, **⌘K** to find one.",
	},
}

-- Conversations, newest first by `updated`, and each one's messages.
M.conversations = ui.persist(
	"conversations",
	{
		{ id = "c1", title = "Welcome", updated = 1 },
	} :: { Conversation }
)
M.messages = ui.persist("messages", { c1 = welcome } :: { [string]: { Message } })
M.current = ui.persist("current", "c1")
-- The conversation shown's messages.
M.thread = ui.computed(function(): { Message }
	return M.messages:get()[M.current:get()] or {}
end)
-- What is being typed, in each conversation.
M.drafts = ui.state("drafts", {} :: { [string]: string })
-- The conversation a reply streams into, if one does.
M.streaming = ui.state("streaming", nil :: string?)
-- The sidebar's search.
M.query = ui.state("query", "")
-- Ids made in this run: after the persisted ones.
M.made = ui.persist("made", 2)

function M.new_id(prefix: string): string
	M.made:add(1)
	return `{prefix}{M.made:peek()}`
end

-- Refs to move focus by keyboard.
M.composer = ui.ref()
M.search = ui.ref()
M.transcript = ui.ref()

return M
