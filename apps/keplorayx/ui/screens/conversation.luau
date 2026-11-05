--!strict
-- The open thread as Zeron's transcript reads it (ds/transcript, vendored;
-- MIT, see THIRD_PARTY_NOTICES.md): what you sent, then each reply as its
-- parts in order, its text and, between, each run of tools and thoughts as
-- one group. The block being written stays apart, a small state the
-- transcript reads for that one block, so a streamed word writes no list.
--
--   local conversation = require("@screens/conversation")
--   Transcript { messages = conversation.messages, draft = conversation.draft }

local store = require("@kx/store")

local M = {}

type Entry = store.Entry

-- Our tools by name, as the transcript's kinds and verbs.
local KIND: { [string]: string } = {
	Bash = "exec",
	Read = "read",
	Edit = "edit",
	MultiEdit = "edit",
	Write = "write",
	Grep = "search",
	Glob = "glob",
	WebFetch = "web",
	WebSearch = "web",
	TodoWrite = "todo",
	Task = "agent",
}

local function tool_of(e: Entry): any
	if e.kind == "thinking" then
		return { id = e.id, kind = "thought", verb = "Thought", call = e.text, prose = true }
	end
	local t = e.tool :: store.Tool
	local added, removed = 0, 0
	for _, c in t.changes do
		added += c.added
		removed += c.removed
	end
	-- Our line is "<Verb> <what>": the verb, and what it acted on.
	local verb, target = string.match(e.text, "^(%S+)%s+(.+)$")
	return {
		id = e.id,
		kind = KIND[t.name] or "exec",
		verb = verb or t.name,
		target = target or e.text,
		added = if added > 0 then added else nil,
		removed = if removed > 0 then removed else nil,
		failed = t.state == "failed",
		live = t.state == "running",
		output = if t.output ~= "" then t.output else nil,
	}
end

-- Which block is being written: its id alone, so the list is made again
-- as a block begins or ends, never as a word arrives.
local draft_id = ui.computed(function(): string
	return store.draft:get().id
end)

local function is_step(e: Entry): boolean
	return e.kind == "tool" or e.kind == "thinking"
end

-- A display tool's call (Keplorayx's `show_visual`, `show_image`, however
-- the agent's harness spells the name): what it showed, from the line its
-- result ends with (src/mcp.rs), as the transcript's block. Nil for any
-- other tool, or one that failed (that stays a tool row, its error in it).
local function shown(e: Entry): any?
	local t = e.tool
	if e.kind ~= "tool" or not t or t.state == "failed" then
		return nil
	end
	local name = t.name
	local visual = string.find(name, "show_visual$") ~= nil
	local image = string.find(name, "show_image$") ~= nil
	if not visual and not image then
		return nil
	end
	local kind, id, a, b = string.match(t.output, "%[(%a+) (%S+) | ([^|]*) |? ?([^%]]*)%]%s*$")
	if visual and kind == "visual" and id then
		-- (A fixture's carries its script.)
		return { id = id, kind = "visual", title = string.match(a, "^%s*(.-)%s*$"), path = b, source = (t :: any).source }
	elseif image and kind == "image" and id then
		return { id = "img:" .. e.id, kind = "images", images = { { id = id, path = a, name = "" } } }
	end
	return nil
end

-- The messages, made again as an entry comes, goes or changes (never as a
-- word streams: that's `draft`).
M.messages = ui.computed(function(): { any }
	local out: { any } = {}
	local reply: any = nil
	local group: any = nil
	local function ensure_reply(id: string)
		if not reply then
			reply = { id = "reply:" .. id, role = "assistant", text = "", blocks = {}, tools = {} }
			table.insert(out, reply)
		end
	end
	for _, e in store.entries:get() do
		if e.kind == "user" then
			reply, group = nil, nil
			table.insert(out, { id = e.id, role = "user", text = e.text })
		elseif shown(e) then
			ensure_reply(e.id)
			group = nil
			table.insert(reply.blocks, shown(e))
		elseif is_step(e) then
			ensure_reply(e.id)
			if not group then
				group = { id = "tools:" .. e.id, kind = "tools", tools = {} }
				table.insert(reply.blocks, group)
			end
			local t = tool_of(e)
			table.insert(group.tools, t)
			table.insert(reply.tools, t)
		else
			ensure_reply(e.id)
			group = nil
			local text = e.text
			if e.kind == "error" then
				text = `> **Error:** {e.text}`
			end
			table.insert(reply.blocks, { id = e.id, kind = "text", text = text })
			reply.text = if reply.text == "" then text else reply.text .. "\n\n" .. text
		end
	end
	-- The turn under way: the block being written joins the last reply (or
	-- begins one), and the reply streams; or, nothing come back yet, the
	-- prompt says the agent is working.
	local t = store.thread:get()
	local running = t ~= nil and t.running
	local writing = draft_id:get()
	if writing ~= "" then
		if not reply then
			reply = { id = "reply:" .. writing, role = "assistant", text = "", blocks = {}, tools = {} }
			table.insert(out, reply)
		end
		table.insert(reply.blocks, { id = "draft:" .. writing, kind = "text", text = "" })
	end
	if reply and running then
		reply.streaming = true
	elseif running and #out > 0 and out[#out].role == "user" then
		out[#out].waiting = "working"
		out[#out].since = ui.now() / 1000
	end
	return out
end)

-- The words being written, for the one block that grows.
M.draft = ui.computed(function(): any
	local d = store.draft:get()
	local list = M.messages:peek()
	local last = list[#list]
	if d.id == "" or not last or last.role ~= "assistant" then
		return { id = "", text = "" }
	end
	return { id = last.id, text = d.text, part = "draft:" .. d.id, part_text = d.text }
end)

return M
