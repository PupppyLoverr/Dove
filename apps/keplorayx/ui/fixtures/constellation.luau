--!strict
-- Constellation with three projects' threads in every state.
local common = require("@fixtures/common")
return { projects = common.projects, agents = common.agents, threads = common.threads, route = "constellation" }
