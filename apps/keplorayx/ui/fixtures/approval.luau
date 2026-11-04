--!strict
-- A thread waiting on you: the agent asks to run a command.
local common = require("@fixtures/common")

return {
	projects = common.projects,
	agents = common.agents,
	threads = common.threads,
	thread = { id = "t2", title = "Approve: run the store migration", harness = "Codex", running = true, status = "Waiting for you" },
	entries = {
		{ id = "u1", kind = "user", text = "Add an index on tasks.updated_at and migrate the store." },
		{
			id = "t1",
			kind = "tool",
			text = "Edit crates/dove-store/src/lib.rs",
			tool = {
				name = "Edit",
				state = "done",
				output = "",
				changes = { { path = "crates/dove-store/src/lib.rs", added = 18, removed = 2 } },
			},
		},
		{ id = "a1", kind = "assistant", text = "The migration is written. It needs to run against your local store once." },
	},
	approval = {
		request_id = "r1",
		tool = "Bash",
		summary = "cargo run -p dove-store --example migrate -- --home %LOCALAPPDATA%\\keplorayx-zgx-dev",
		questions = {},
	},
}
