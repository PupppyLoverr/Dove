--!strict
-- The Inbox: one agent waiting, one thread finished and unread.
local common = require("@fixtures/common")
return { projects = common.projects, agents = common.agents, threads = common.threads, route = "inbox" }
