--!strict
-- The thread with the panel open on pulls.
local thread = require("@fixtures/thread")
local f = table.clone(thread :: any)
f.panel_tab = "pulls"
return f
