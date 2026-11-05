--!strict
-- The thread with the panel open on its home: what can open here.
local thread = require("@fixtures/thread")
local f = table.clone(thread :: any)
f.panel_tab = "home"
return f
