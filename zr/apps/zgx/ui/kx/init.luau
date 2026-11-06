--!strict
-- Keplorayx's own pieces (its island, launcher and widgets) drawn with
-- Zeron's design system: the few names they were written against (kx.Icon,
-- kx.Button, kx.StatusIcon, kx.AgentMark, kx.toast, kx.motion…) as Zeron's
-- components, so what moved onto Zeron's app keeps its shape. Its colours
-- are the theme's, Keplorayx's Orbit laid over Zeron's (ds/themes).

local ds = require("@ds")
local status = require("@ds/status")
local AgentMark = require("@ds/agent_mark")

local M = {}

M.Icon = ds.Icon

-- `Kbd { "Alt" }`: a key in its cap.
function M.Kbd(props: { [any]: any }): Node
	return ds.Kbd { keys = props[1] or props.keys or "" }
end

function M.Spinner(props: { size: number? }?): Node
	return status.Spinner { size = (props or {}).size }
end

-- What Keplorayx called a thread's state, in Zeron's kinds.
local KIND: { [string]: string } = {
	running = "working",
	working = "working",
	needs_you = "input",
	input = "input",
	approval = "approval",
	done = "done",
	failed = "failed",
	idle = "done",
}
function M.StatusIcon(props: { kind: any, size: number? }): Node
	local kind = props.kind
	return status.StatusIcon {
		kind = if type(kind) == "function"
			then function()
				return KIND[kind()] or kind()
			end
			else (KIND[kind] or kind) :: any,
		size = props.size,
	}
end

-- An agent's mark by its harness id ("claude_code") or Zeron's mark name.
local MARK: { [string]: string } = {
	claude_code = "claude",
	codex = "openai",
	cursor = "cursor",
	grok = "grok",
	devin = "devin",
	open_code = "opencode",
	opencode = "opencode",
	antigravity = "antigravity",
	pi = "pi",
	hermes = "hermes",
	gemini = "gemini",
}
function M.AgentMark(props: { agent: string?, size: number?, live: any? }): Node
	local agent = props.agent or ""
	return AgentMark.Mark {
		label = agent,
		mark = MARK[agent] or agent,
		seed = agent,
		size = props.size or 16,
		bare = true,
	}
end

-- Keplorayx's tones, as Zeron's: `attention` is the primary.
local TONE: { [string]: string } = { attention = "primary", primary = "primary", secondary = "secondary", ghost = "ghost" }
function M.Button(props: { [any]: any }): Node
	local p = table.clone(props)
	p.tone = TONE[props.tone or "primary"] or "primary"
	return ds.Button(p)
end

function M.toast(t: { title: string, body: string?, kind: string? })
	ds.toast.show({ title = t.title, body = t.body, kind = t.kind } :: any)
end

-- The one breath every live thing shares (as Keplorayx's kx.motion).
local BREATH_MS = 2400
local phase = ui.state("kx.breath", 0)
local holders = 0
local timer: any = nil
local function hold()
	holders += 1
	if timer == nil then
		timer = anime.createTimer {
			loop = true,
			duration = BREATH_MS,
			frameRate = 30,
			onUpdate = function(a: any)
				local t = (a.iterationCurrentTime or 0) / BREATH_MS
				phase:set(0.5 - 0.5 * math.cos(t * 2 * math.pi))
			end,
		}
	end
end
local function release()
	holders = math.max(0, holders - 1)
	if holders == 0 and timer ~= nil then
		timer:cancel()
		timer = nil
	end
end
M.motion = {
	EXIT = "cubicBezier(0.4, 0, 1, 1)",
	breath = function(low: number, high: number): () -> number
		hold()
		ui.on_cleanup(release)
		return function()
			return low + (high - low) * phase:get()
		end
	end,
}

function M.setup() end

function M.float_style(p: { [any]: any }): { [any]: any }
	return p
end

return M
