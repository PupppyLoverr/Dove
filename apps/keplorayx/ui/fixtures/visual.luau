--!strict
-- A reply with an agent's visual (`show_visual`): its tool call drawn as
-- the visual itself, between the reply's text.
local common = require("@fixtures/common")

local script = [[
local s = ui.state { metric = "p95" }
local data = {
  { region = "us-east", p95 = 182, p99 = 240 },
  { region = "eu-west", p95 = 140, p99 = 210 },
  { region = "ap-south", p95 = 221, p99 = 305 },
}
return function()
  return ui.column { gap = 16,
    ui.row { gap = 32,
      ui.stat { value = "182 ms", label = "p95, us-east", delta = "-12%", tone = "success" },
      ui.stat { value = "305 ms", label = "worst p99", delta = "+4%", tone = "danger" },
    },
    ui.text { "Latency by region", size = "lg", weight = 600 },
    ui.tabs { options = { "p95", "p99" }, value = s.metric, on_change = function(v) s.metric = v end },
    ui.chart { type = "bar", height = 200, unit = "ms",
      x = ui.map(data, function(d) return d.region end),
      series = { { name = s.metric, values = ui.map(data, function(d) return d[s.metric] end) } } },
  }
end]]

return {
	projects = common.projects,
	agents = common.agents,
	threads = common.threads,
	thread = { id = "t1", title = "Why is ap-south slow?", harness = "Claude Code", running = false, status = "" },
	entries = {
		{ id = "u1", kind = "user", text = "Compare latency across regions for me." },
		{
			id = "t1",
			kind = "tool",
			text = "show_visual Latency",
			tool = {
				name = "mcp__keplorayx__show_visual",
				state = "done",
				output = "Shown above your reply (9 nodes, 2 controls checked).\n[visual v-fixture | Latency | fixture]",
				changes = {},
				source = script,
			},
		},
		{ id = "a1", kind = "assistant", text = "ap-south is the outlier: its p99 runs **305 ms**, about 30% over the others. Its traffic crosses an extra hop through the shared gateway." },
	},
	draft = { id = "", kind = "", text = "" },
}
