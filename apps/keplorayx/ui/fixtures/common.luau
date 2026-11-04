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

-- What the host's calls would answer (kx/store's `call`).
local ROOT = {
	{ name = "apps", path = "apps", dir = true, size = 0, status = "M" },
	{ name = "crates", path = "crates", dir = true, size = 0, status = "" },
	{ name = "docs", path = "docs", dir = true, size = 0, status = "" },
	{ name = "Cargo.toml", path = "Cargo.toml", dir = false, size = 2210, status = "" },
	{ name = "README.md", path = "README.md", dir = false, size = 5120, status = "M" },
	{ name = "STATUS.md", path = "STATUS.md", dir = false, size = 9800, status = "?" },
}
local APPS = {
	{ name = "keplorayx", path = "apps/keplorayx", dir = true, size = 0, status = "M" },
}
local SOURCE = [[//! The notch: a capsule at the top centre of the screen.

use std::sync::Arc;

/// The capsule's window, in points.
pub const SIZE: (f64, f64) = (460., 56.);

pub fn enabled() -> bool {
    std::env::var_os("KX_NOTCH").is_none_or(|v| v != "0")
}
]]

local function lanes(t: { number }): { number }
	return t
end

M.stubs = {
	["kx.list_dir"] = function(arg: any)
		return if arg.path == "apps" then APPS else ROOT
	end,
	["kx.read_file"] = function(_arg: any)
		return { text = SOURCE, lang = "rust", size = #SOURCE, binary = false, truncated = false, kind = "text", full = "" }
	end,
	["kx.history"] = function()
		return {
			{ sha = "a1", short = "b37f8e6", subject = "Cargo.lock: keplorayx depends on dove-git", author = "You", when = "2 hours ago", refs = { "HEAD -> zgx/phase-a" }, head = true, lane = 0, top = lanes({}), bottom = lanes({ 0 }), joins = {}, forks = {} },
			{ sha = "a2", short = "346574b", subject = "Dawn holds with the notch open", author = "You", when = "3 hours ago", refs = {}, head = false, lane = 0, top = { 0 }, bottom = { 0, 1 }, joins = {}, forks = { 1 } },
			{ sha = "a3", short = "59ccf96", subject = "First Light, Constellation, the terminal", author = "You", when = "5 hours ago", refs = {}, head = false, lane = 1, top = { 0, 1 }, bottom = { 0, 1 }, joins = {}, forks = {} },
			{ sha = "a4", short = "a822738", subject = "Phase D begins: the Changes panel", author = "You", when = "7 hours ago", refs = {}, head = false, lane = 0, top = { 0, 1 }, bottom = { 0 }, joins = { 1 }, forks = {} },
			{ sha = "a5", short = "8a2a695", subject = "Phase C: the shell, Home, threads", author = "You", when = "yesterday", refs = { "tag: v0.1" }, head = false, lane = 0, top = { 0 }, bottom = { 0 }, joins = {}, forks = {} },
		}
	end,
	["kx.pulls"] = function()
		return {
			{ number = 128, title = "Orbit: Zeron's look for the shell", author = "you", branch = "zgx/phase-a", state = "OPEN", draft = false, url = "", updated = "today", review = "REVIEW_REQUIRED", added = 4210, removed = 1180 },
			{ number = 127, title = "Notch quick replies", author = "you", branch = "notch-replies", state = "OPEN", draft = true, url = "", updated = "yesterday", review = "", added = 220, removed = 31 },
		}
	end,
	["kx.team"] = function()
		return {
			agents = {
				{ id = "claude_code", label = "Claude Code", installed = true },
				{ id = "codex", label = "Codex", installed = true },
				{ id = "gemini", label = "Gemini CLI", installed = true },
				{ id = "cursor", label = "Cursor", installed = false },
				{ id = "open_code", label = "OpenCode", installed = true },
			},
			members = {
				{ id = "m1", name = "Iris", role = "Reviewer", avatar = "", hue = 0.6, harness = "codex", instructions = "Reads every diff twice. Asks for a test before anything ships." },
				{ id = "m2", name = "Theo", role = "Frontend lead", avatar = "", hue = 0.3, harness = "claude_code", instructions = "Keeps the interface calm and fast; no layout in a hot path." },
			},
		}
	end,
	["kx.settings"] = function()
		return { ["kx.global_keys"] = "true" }
	end,
}

return M
