--!strict
-- The thread with the panel open on browser.
local thread = require("@fixtures/thread")
local f = table.clone(thread :: any)
f.panel_tab = "browser"
return f
