--!strict
-- The thread with the panel open on its Changes.
local thread = require("@fixtures/thread")
local f = table.clone(thread :: any)
f.panel_tab = "changes"
return f
