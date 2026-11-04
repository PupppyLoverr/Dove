--!strict
-- The notch with an agent waiting on you.
local common = require("@fixtures/common")
return {
	projects = common.projects,
	agents = common.agents,
	threads = common.threads,
	attention = {
		{
			thread_id = "t2",
			title = "Approve: run the store migration",
			harness = "Codex",
			harness_id = "codex",
			request_id = "r1",
			tool = "Bash",
			summary = "cargo run -p dove-store --example migrate",
		},
	},
}
