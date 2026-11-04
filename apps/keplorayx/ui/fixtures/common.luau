--!strict
-- What the fixtures share: a few projects, the agents, and threads in
-- every state. Times are fixed so the PNGs don't drift.

local NOW = 1762300000000
local MIN = 60000

local M = {}

M.projects = {
	{ id = "p1", name = "keplorayx", path = "C:\\dev\\keplorayx-zgx" },
	{ id = "p2", name = "woozlit-web", path = "C:\\dev\\woozlit-web" },
	{ id = "p3", name = "notes", path = "C:\\Users\\ravth\\notes" },
}

M.agents = {
	{ id = "claude_code", label = "Claude Code" },
	{ id = "codex", label = "Codex" },
	{ id = "gemini", label = "Gemini CLI" },
	{ id = "open_code", label = "OpenCode" },
}

local function thread(id: string, title: string, p: number, agent: string, label: string, status: string, ago: number, unread: boolean?)
	local project = M.projects[p]
	return {
		id = id,
		title = title,
		project = project.name,
		project_id = project.id,
		status = status,
		harness = label,
		harness_id = agent,
		updated = NOW - ago * MIN,
		unread = unread == true,
	}
end

M.threads = {
	thread("t1", "Port the notch to a zgx overlay", 1, "claude_code", "Claude Code", "running", 1),
	thread("t2", "Approve: run the store migration", 1, "codex", "Codex", "needs_you", 3),
	thread("t3", "Tidy the settings pages", 1, "gemini", "Gemini CLI", "done", 42, true),
	thread("t4", "Fix the flaky sign-in redirect", 2, "codex", "Codex", "running", 6),
	thread("t5", "Summarise last week's PRs", 2, "open_code", "OpenCode", "done", 180),
	thread("t6", "Outline the onboarding copy", 3, "claude_code", "Claude Code", "done", 1440),
}

M.NOW = NOW

return M
