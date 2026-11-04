--!strict
-- Home with work under way: three projects, agents running, one waiting.
local common = require("@fixtures/common")

return {
	projects = common.projects,
	agents = common.agents,
	threads = common.threads,
	thread = nil,
}
