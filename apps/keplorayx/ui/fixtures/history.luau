--!strict
-- The thread with the panel open on history.
local thread = require("@fixtures/thread")
local f = table.clone(thread :: any)
f.panel_tab = "history"
return f
