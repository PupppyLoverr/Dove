--!strict
-- A terminal in the open thread's project: one shell a project, kept
-- running across reloads and thread switches (zgx's terminal sessions
-- belong to the host). Drawn in the mono face on the panel's own ground.

local kx = require("@kx")
local store = require("@kx/store")

local M = {}

-- The open thread's project folder, or none.
local project = ui.computed(function(): { id: string, path: string }?
	local t = store.thread:get()
	if not t then
		return nil
	end
	local pid: string? = nil
	for _, th in store.threads:get() do
		if th.id == t.id then
			pid = th.project_id
		end
	end
	for _, p in store.projects:get() do
		if p.id == pid then
			return { id = p.id, path = p.path }
		end
	end
	return nil
end)

local function shell(p: { id: string, path: string }): Node
	local term = ui.term(`project:{p.id}`, { cwd = p.path, font_size = 12.5 })
	return ui.terminal {
		term = term,
		grow = 1,
		w_full = true,
		min_h = 0,
		p = "$space.2",
	}
end

function M.view(): Node
	if store.fixture ~= "" then
		-- Headless: no shell to start, a still of what one looks like.
		return ui.column {
			grow = 1,
			w_full = true,
			p = "$space.3",
			gap = "$kx.space.hair",
			kx.Mono { "PS C:\\dev\\keplorayx-zgx> cargo check -p keplorayx", literal = true },
			kx.Mono { "    Finished `dev` profile in 12.4s", literal = true, color = "$success" },
			kx.Mono { "PS C:\\dev\\keplorayx-zgx> ", literal = true },
		}
	end
	return ui.presence(function()
		local p = project:get()
		return if p then p.id else nil
	end, function(_id: string)
		local p = project:peek()
		if not p then
			return kx.Empty { icon = "terminal", title = "No project", hint = "Open a thread to get a shell in its project." }
		end
		return shell(p)
	end, { grow = 1, w_full = true, min_h = 0 })
end

return M
